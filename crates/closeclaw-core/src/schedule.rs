use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Information about a schedule entry, used for listing and persistence.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleInfo {
    pub id: String,
    pub cron: String,
    pub agent_id: String,
    pub message: String,
    /// "config" for entries from config file, "dynamic" for runtime-created entries.
    pub source: String,
    /// Peer ID of the user who created this schedule (e.g. "tg:12345").
    /// Used to deliver scheduled responses back to the originating channel/chat.
    #[serde(default)]
    pub notify_peer_id: Option<String>,
}

/// Handle for managing schedules at runtime. Implemented in the gateway crate,
/// consumed by tools in the tools crate.
#[async_trait]
pub trait ScheduleHandle: Send + Sync {
    async fn add_schedule(
        &self,
        id: String,
        cron: String,
        agent_id: String,
        message: String,
    ) -> Result<String, String>;

    async fn remove_schedule(&self, id: &str) -> Result<String, String>;

    async fn list_schedules(&self) -> Result<Vec<ScheduleInfo>, String>;
}

/// Callback for delivering scheduled task responses to channels (e.g. Telegram).
#[async_trait]
pub trait ScheduleNotifier: Send + Sync {
    /// Called after a scheduled task fires and the agent produces a response.
    async fn notify(&self, schedule_id: &str, peer_id: &str, response: &str);
}

tokio::task_local! {
    /// The peer ID of the user whose message is currently being processed.
    /// Set by the Hub before agent execution, read by ScheduleHandleImpl.
    pub static CURRENT_PEER_ID: String;
}
