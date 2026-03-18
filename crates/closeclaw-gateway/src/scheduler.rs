use std::str::FromStr;
use std::sync::Arc;

use async_trait::async_trait;
use chrono::Local;
use cron::Schedule;
use tokio::sync::{mpsc, oneshot, watch};
use tracing::{error, info, warn};

use closeclaw_core::config::ScheduleConfig;
use closeclaw_core::schedule::{ScheduleHandle, ScheduleInfo, ScheduleNotifier, CURRENT_PEER_ID};
use closeclaw_core::types::{AgentId, ChannelId, Message, SessionId};

use crate::hub::Hub;
use crate::schedule_store::ScheduleStore;

struct ScheduleEntry {
    id: String,
    cron_expr: String,
    schedule: Schedule,
    agent_id: String,
    message: String,
    /// "config" or "dynamic"
    source: String,
    /// Peer ID for notification delivery (e.g. "tg:12345").
    notify_peer_id: Option<String>,
}

/// Commands sent from tools to the scheduler via mpsc channel.
pub enum ScheduleCommand {
    Add {
        id: String,
        cron: String,
        agent_id: String,
        message: String,
        notify_peer_id: Option<String>,
        reply: oneshot::Sender<Result<String, String>>,
    },
    Remove {
        id: String,
        reply: oneshot::Sender<Result<String, String>>,
    },
    List {
        reply: oneshot::Sender<Result<Vec<ScheduleInfo>, String>>,
    },
}

pub struct Scheduler {
    entries: Vec<ScheduleEntry>,
    cmd_rx: mpsc::Receiver<ScheduleCommand>,
    schedule_store: ScheduleStore,
    notifier: Option<Arc<dyn ScheduleNotifier>>,
}

impl Scheduler {
    /// Create a scheduler from config-file entries + persisted dynamic entries.
    pub fn new(
        configs: &[ScheduleConfig],
        store: ScheduleStore,
        cmd_rx: mpsc::Receiver<ScheduleCommand>,
        notifier: Option<Arc<dyn ScheduleNotifier>>,
    ) -> Self {
        let mut entries = Vec::new();

        // Load config-file entries
        for cfg in configs {
            if !cfg.enabled {
                info!(id = %cfg.id, "Schedule disabled, skipping");
                continue;
            }
            match Schedule::from_str(&cfg.cron) {
                Ok(schedule) => {
                    info!(id = %cfg.id, cron = %cfg.cron, source = "config", "Loaded schedule");
                    entries.push(ScheduleEntry {
                        id: cfg.id.clone(),
                        cron_expr: cfg.cron.clone(),
                        schedule,
                        agent_id: cfg.agent_id.clone(),
                        message: cfg.message.clone(),
                        source: "config".to_string(),
                        notify_peer_id: None,
                    });
                }
                Err(e) => {
                    warn!(id = %cfg.id, cron = %cfg.cron, error = %e, "Invalid cron expression, skipping");
                }
            }
        }

        // Load persisted dynamic entries
        for info in store.load() {
            // Skip if a config entry already has this ID
            if entries.iter().any(|e| e.id == info.id) {
                warn!(id = %info.id, "Dynamic schedule ID conflicts with config entry, skipping");
                continue;
            }
            match Schedule::from_str(&info.cron) {
                Ok(schedule) => {
                    info!(id = %info.id, cron = %info.cron, source = "dynamic", "Loaded persisted schedule");
                    entries.push(ScheduleEntry {
                        id: info.id,
                        cron_expr: info.cron,
                        schedule,
                        agent_id: info.agent_id,
                        message: info.message,
                        source: "dynamic".to_string(),
                        notify_peer_id: info.notify_peer_id,
                    });
                }
                Err(e) => {
                    warn!(id = %info.id, cron = %info.cron, error = %e, "Invalid persisted cron, skipping");
                }
            }
        }

        Self {
            entries,
            cmd_rx,
            schedule_store: store,
            notifier,
        }
    }

    /// Pre-register all schedule sessions on the Hub so that deterministic
    /// session IDs are reused and history is restored from disk.
    pub async fn restore_sessions(&self, hub: &Hub) {
        for entry in &self.entries {
            let session_id = SessionId(format!("sched-{}", entry.id));
            let channel_id = ChannelId("scheduler".to_string());
            let agent_id = AgentId(entry.agent_id.clone());

            if let Err(e) = hub
                .restore_session(channel_id, "scheduler".to_string(), agent_id, session_id)
                .await
            {
                warn!(id = %entry.id, error = %e, "Failed to restore schedule session");
            }
        }
    }

    /// Persist only dynamic entries to disk.
    fn persist_dynamic(&self) {
        let dynamic: Vec<ScheduleInfo> = self
            .entries
            .iter()
            .filter(|e| e.source == "dynamic")
            .map(|e| ScheduleInfo {
                id: e.id.clone(),
                cron: e.cron_expr.clone(),
                agent_id: e.agent_id.clone(),
                message: e.message.clone(),
                source: e.source.clone(),
                notify_peer_id: e.notify_peer_id.clone(),
            })
            .collect();
        if let Err(e) = self.schedule_store.save(&dynamic) {
            error!(error = %e, "Failed to persist dynamic schedules");
        }
    }

    /// Run the scheduler loop. Ticks every second, fires matching entries as
    /// independent tokio tasks. Also listens for commands from agent tools.
    /// Stops when `shutdown_rx` receives a signal.
    pub async fn run(mut self, hub: Arc<Hub>, mut shutdown_rx: watch::Receiver<bool>) {
        info!(count = self.entries.len(), "Scheduler started");

        // Compute initial next-fire times
        let mut next_fires: Vec<Option<chrono::DateTime<Local>>> = self
            .entries
            .iter()
            .map(|e| e.schedule.upcoming(Local).next())
            .collect();

        // Log initial next-fire times
        for (entry, nf) in self.entries.iter().zip(next_fires.iter()) {
            if let Some(t) = nf {
                info!(id = %entry.id, next_fire = %t, "Next fire time");
            }
        }

        let mut interval = tokio::time::interval(tokio::time::Duration::from_secs(1));

        loop {
            tokio::select! {
                _ = interval.tick() => {
                    self.tick_entries(&hub, &mut next_fires);
                }
                cmd = self.cmd_rx.recv() => {
                    match cmd {
                        Some(cmd) => self.handle_command(cmd, &hub, &mut next_fires).await,
                        None => {
                            info!("Schedule command channel closed, scheduler stopping");
                            return;
                        }
                    }
                }
                _ = shutdown_rx.changed() => {
                    info!("Scheduler shutting down");
                    return;
                }
            }
        }
    }

    fn tick_entries(
        &self,
        hub: &Arc<Hub>,
        next_fires: &mut Vec<Option<chrono::DateTime<Local>>>,
    ) {
        let now = Local::now();

        for (i, entry) in self.entries.iter().enumerate() {
            let should_fire = match next_fires[i] {
                Some(nf) => now >= nf,
                None => false,
            };

            if should_fire {
                info!(id = %entry.id, "Firing scheduled task");

                // Advance to next fire time
                next_fires[i] = entry.schedule.upcoming(Local).next();

                let hub = hub.clone();
                let id = entry.id.clone();
                let message_text = entry.message.clone();
                let notifier = self.notifier.clone();
                let notify_peer_id = entry.notify_peer_id.clone();

                tokio::spawn(async move {
                    fire_schedule(&hub, &id, &message_text, notifier.as_deref(), notify_peer_id.as_deref()).await;
                });
            }
        }
    }

    async fn handle_command(
        &mut self,
        cmd: ScheduleCommand,
        hub: &Arc<Hub>,
        next_fires: &mut Vec<Option<chrono::DateTime<Local>>>,
    ) {
        match cmd {
            ScheduleCommand::Add {
                id,
                cron,
                agent_id,
                message,
                notify_peer_id,
                reply,
            } => {
                let result = self.handle_add(id, cron, agent_id, message, notify_peer_id, hub, next_fires).await;
                let _ = reply.send(result);
            }
            ScheduleCommand::Remove { id, reply } => {
                let result = self.handle_remove(&id, next_fires);
                let _ = reply.send(result);
            }
            ScheduleCommand::List { reply } => {
                let result = self.handle_list();
                let _ = reply.send(result);
            }
        }
    }

    async fn handle_add(
        &mut self,
        id: String,
        cron: String,
        agent_id: String,
        message: String,
        notify_peer_id: Option<String>,
        hub: &Arc<Hub>,
        next_fires: &mut Vec<Option<chrono::DateTime<Local>>>,
    ) -> Result<String, String> {
        // Validate cron
        let schedule = Schedule::from_str(&cron).map_err(|e| format!("Invalid cron expression: {e}"))?;

        // Reject duplicate ID
        if self.entries.iter().any(|e| e.id == id) {
            return Err(format!("Schedule with id '{id}' already exists"));
        }

        // Restore session on hub
        let session_id = SessionId(format!("sched-{}", id));
        let channel_id = ChannelId("scheduler".to_string());
        let agent_id_typed = AgentId(agent_id.clone());

        if let Err(e) = hub
            .restore_session(channel_id, "scheduler".to_string(), agent_id_typed, session_id)
            .await
        {
            warn!(id = %id, error = %e, "Failed to restore session for new schedule");
        }

        let next = schedule.upcoming(Local).next();
        info!(id = %id, cron = %cron, notify_peer_id = ?notify_peer_id, next_fire = ?next, "Added dynamic schedule");

        self.entries.push(ScheduleEntry {
            id: id.clone(),
            cron_expr: cron.clone(),
            schedule,
            agent_id,
            message,
            source: "dynamic".to_string(),
            notify_peer_id,
        });
        next_fires.push(next);

        self.persist_dynamic();

        let next_str = next.map(|t| t.to_string()).unwrap_or_else(|| "never".to_string());
        Ok(format!("Schedule '{id}' added. Next fire: {next_str}"))
    }

    fn handle_remove(
        &mut self,
        id: &str,
        next_fires: &mut Vec<Option<chrono::DateTime<Local>>>,
    ) -> Result<String, String> {
        let pos = self
            .entries
            .iter()
            .position(|e| e.id == id)
            .ok_or_else(|| format!("Schedule '{id}' not found"))?;

        if self.entries[pos].source == "config" {
            return Err(format!(
                "Schedule '{id}' is from the config file and cannot be removed dynamically. \
                 Edit config.toml to remove it."
            ));
        }

        self.entries.remove(pos);
        next_fires.remove(pos);
        self.persist_dynamic();

        info!(id = %id, "Removed dynamic schedule");
        Ok(format!("Schedule '{id}' removed"))
    }

    fn handle_list(&self) -> Result<Vec<ScheduleInfo>, String> {
        let infos = self
            .entries
            .iter()
            .map(|e| ScheduleInfo {
                id: e.id.clone(),
                cron: e.cron_expr.clone(),
                agent_id: e.agent_id.clone(),
                message: e.message.clone(),
                source: e.source.clone(),
                notify_peer_id: e.notify_peer_id.clone(),
            })
            .collect();
        Ok(infos)
    }
}

/// Implementation of ScheduleHandle that sends commands to the scheduler via mpsc.
/// Automatically captures the current peer ID via task-local storage.
pub struct ScheduleHandleImpl {
    tx: mpsc::Sender<ScheduleCommand>,
}

impl ScheduleHandleImpl {
    pub fn new(tx: mpsc::Sender<ScheduleCommand>) -> Self {
        Self { tx }
    }
}

#[async_trait]
impl ScheduleHandle for ScheduleHandleImpl {
    async fn add_schedule(
        &self,
        id: String,
        cron: String,
        agent_id: String,
        message: String,
    ) -> Result<String, String> {
        // Auto-capture the peer ID from the task-local context (set by Hub)
        let notify_peer_id = CURRENT_PEER_ID.try_with(|p| p.clone()).ok();

        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ScheduleCommand::Add {
                id,
                cron,
                agent_id,
                message,
                notify_peer_id,
                reply: reply_tx,
            })
            .await
            .map_err(|_| "Scheduler is not running".to_string())?;
        reply_rx
            .await
            .map_err(|_| "Scheduler dropped the reply".to_string())?
    }

    async fn remove_schedule(&self, id: &str) -> Result<String, String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ScheduleCommand::Remove {
                id: id.to_string(),
                reply: reply_tx,
            })
            .await
            .map_err(|_| "Scheduler is not running".to_string())?;
        reply_rx
            .await
            .map_err(|_| "Scheduler dropped the reply".to_string())?
    }

    async fn list_schedules(&self) -> Result<Vec<ScheduleInfo>, String> {
        let (reply_tx, reply_rx) = oneshot::channel();
        self.tx
            .send(ScheduleCommand::List { reply: reply_tx })
            .await
            .map_err(|_| "Scheduler is not running".to_string())?;
        reply_rx
            .await
            .map_err(|_| "Scheduler dropped the reply".to_string())?
    }
}

async fn fire_schedule(
    hub: &Hub,
    schedule_id: &str,
    message_text: &str,
    notifier: Option<&dyn ScheduleNotifier>,
    notify_peer_id: Option<&str>,
) {
    let session_id = SessionId(format!("sched-{}", schedule_id));
    let channel_id = ChannelId("scheduler".to_string());

    let msg = Message::user_text(
        session_id,
        channel_id,
        "scheduler",
        "scheduler",
        message_text,
    );

    match hub.handle_message(msg).await {
        Ok(response) => {
            info!(
                schedule_id = %schedule_id,
                response_len = response.len(),
                "Schedule fired successfully"
            );
            tracing::debug!(schedule_id = %schedule_id, response = %response, "Schedule response");

            // Deliver to the originating channel if a notifier + peer_id are configured
            if let (Some(notifier), Some(peer_id)) = (notifier, notify_peer_id) {
                notifier.notify(schedule_id, peer_id, &response).await;
            }
        }
        Err(e) => {
            error!(schedule_id = %schedule_id, error = %e, "Schedule firing failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use closeclaw_core::config::ScheduleConfig;

    fn make_config(id: &str, cron: &str, enabled: bool) -> ScheduleConfig {
        ScheduleConfig {
            id: id.to_string(),
            cron: cron.to_string(),
            agent_id: "default".to_string(),
            message: "test message".to_string(),
            enabled,
        }
    }

    fn make_scheduler(configs: &[ScheduleConfig]) -> (Scheduler, mpsc::Sender<ScheduleCommand>) {
        let dir = tempfile::tempdir().unwrap();
        let store = ScheduleStore::new(dir.path().join("schedules.json"));
        let (tx, rx) = mpsc::channel(16);
        let scheduler = Scheduler::new(configs, store, rx, None);
        // Leak the tempdir so it's not cleaned up while scheduler is alive
        std::mem::forget(dir);
        (scheduler, tx)
    }

    #[test]
    fn test_valid_cron_parsed() {
        let configs = vec![make_config("test", "0 0 9 * * * *", true)];
        let (scheduler, _tx) = make_scheduler(&configs);
        assert_eq!(scheduler.entries.len(), 1);
        assert_eq!(scheduler.entries[0].id, "test");
        assert_eq!(scheduler.entries[0].source, "config");
    }

    #[test]
    fn test_invalid_cron_skipped() {
        let configs = vec![make_config("bad", "not a cron", true)];
        let (scheduler, _tx) = make_scheduler(&configs);
        assert_eq!(scheduler.entries.len(), 0);
    }

    #[test]
    fn test_disabled_entry_skipped() {
        let configs = vec![make_config("off", "0 0 9 * * * *", false)];
        let (scheduler, _tx) = make_scheduler(&configs);
        assert_eq!(scheduler.entries.len(), 0);
    }

    #[test]
    fn test_empty_configs() {
        let (scheduler, _tx) = make_scheduler(&[]);
        assert_eq!(scheduler.entries.len(), 0);
    }

    #[test]
    fn test_mixed_valid_invalid_disabled() {
        let configs = vec![
            make_config("valid1", "0 0 9 * * * *", true),
            make_config("invalid", "bad cron expr", true),
            make_config("disabled", "0 0 10 * * * *", false),
            make_config("valid2", "0 30 14 * * 1-5 *", true),
        ];
        let (scheduler, _tx) = make_scheduler(&configs);
        assert_eq!(scheduler.entries.len(), 2);
        assert_eq!(scheduler.entries[0].id, "valid1");
        assert_eq!(scheduler.entries[1].id, "valid2");
    }

    #[test]
    fn test_loads_persisted_dynamic_entries() {
        let dir = tempfile::tempdir().unwrap();
        let store_path = dir.path().join("schedules.json");
        let store = ScheduleStore::new(store_path.clone());

        // Pre-persist some dynamic entries
        store
            .save(&[ScheduleInfo {
                id: "dyn1".to_string(),
                cron: "0 0 12 * * * *".to_string(),
                agent_id: "default".to_string(),
                message: "dynamic task".to_string(),
                source: "dynamic".to_string(),
                notify_peer_id: Some("tg:12345".to_string()),
            }])
            .unwrap();

        let store2 = ScheduleStore::new(store_path);
        let (tx, rx) = mpsc::channel(16);
        let scheduler = Scheduler::new(&[], store2, rx, None);
        drop(tx);

        assert_eq!(scheduler.entries.len(), 1);
        assert_eq!(scheduler.entries[0].id, "dyn1");
        assert_eq!(scheduler.entries[0].source, "dynamic");
        assert_eq!(scheduler.entries[0].notify_peer_id.as_deref(), Some("tg:12345"));
    }
}
