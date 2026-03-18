use crate::events::EventBus;
use crate::router::Router;
use crate::session_store::SessionStore;
use closeclaw_agent::runtime::AgentRuntime;
use closeclaw_core::agent::Agent;
use closeclaw_core::error::{CloseClawError, Result};
use closeclaw_core::schedule::CURRENT_PEER_ID;
use closeclaw_core::session::Session;
use closeclaw_core::types::{AgentId, Event, Message, MessageContent, SessionId};
use dashmap::DashMap;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{broadcast, mpsc};
use tracing::info;

pub struct Hub {
    agents: DashMap<AgentId, Arc<AgentRuntime>>,
    sessions: DashMap<SessionId, Session>,
    router: Router,
    session_store: SessionStore,
    event_bus: EventBus,
}

impl Hub {
    pub fn new(
        default_agent_id: AgentId,
        session_dir: PathBuf,
    ) -> Self {
        Self {
            agents: DashMap::new(),
            sessions: DashMap::new(),
            router: Router::new(default_agent_id),
            session_store: SessionStore::new(session_dir),
            event_bus: EventBus::new(1024),
        }
    }

    pub fn register_agent(&self, id: AgentId, runtime: Arc<AgentRuntime>) {
        self.agents.insert(id, runtime);
    }

    pub fn event_sender(&self) -> broadcast::Sender<Event> {
        self.event_bus.sender()
    }

    pub fn subscribe_events(&self) -> broadcast::Receiver<Event> {
        self.event_bus.subscribe()
    }

    /// Process an incoming message from a channel.
    pub async fn handle_message(&self, msg: Message) -> Result<String> {
        let peer_id = match &msg.sender {
            closeclaw_core::types::Sender::User { id, .. } => id.clone(),
            closeclaw_core::types::Sender::Agent { agent_id } => agent_id.0.clone(),
        };

        let (agent_id, session_id, is_new) =
            self.router.resolve(&msg.channel_id, &peer_id);

        if is_new {
            info!("New session {session_id} for agent {agent_id}");
            let session = Session::new(
                session_id.clone(),
                agent_id.clone(),
                msg.channel_id.clone(),
            );
            self.sessions.insert(session_id.clone(), session);
            self.event_bus
                .publish(Event::SessionCreated(session_id.clone()));
        }

        let user_text = match &msg.content {
            MessageContent::Text(t) => t.clone(),
            _ => return Err(CloseClawError::Channel("Only text messages are supported".into())),
        };

        let agent = self
            .agents
            .get(&agent_id)
            .ok_or_else(|| CloseClawError::AgentNotFound(agent_id.to_string()))?
            .clone();

        let mut session = self
            .sessions
            .get_mut(&session_id)
            .ok_or_else(|| CloseClawError::SessionNotFound(session_id.to_string()))?;

        // Create an event sender for the agent
        let (event_tx, mut event_rx) = mpsc::channel(256);

        // Forward events from agent to event bus
        let bus_sender = self.event_bus.sender();
        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                let _ = bus_sender.send(event);
            }
        });

        let response = CURRENT_PEER_ID
            .scope(peer_id, agent.handle_message(&mut session, &user_text, &event_tx))
            .await?;

        // Broadcast agent response
        self.event_bus.publish(Event::AgentResponse {
            session_id: session_id.clone(),
            content: response.clone(),
        });

        Ok(response)
    }

    /// Restore (or create) a session with a deterministic ID. Pre-seeds the
    /// router so future messages from this channel+peer reuse the same session,
    /// and loads any persisted history from disk.
    pub async fn restore_session(
        &self,
        channel_id: closeclaw_core::types::ChannelId,
        peer_id: String,
        agent_id: AgentId,
        session_id: SessionId,
    ) -> Result<()> {
        // Seed the route so handle_message will find it
        self.router.seed(
            channel_id.clone(),
            peer_id,
            agent_id.clone(),
            session_id.clone(),
        );

        // Load history from disk (empty vec if no file exists)
        let history = self.session_store.load_history(&session_id).await?;
        let has_history = !history.is_empty();

        let mut session = Session::new(session_id.clone(), agent_id, channel_id);
        for msg in history {
            session.append(msg);
        }

        self.sessions.insert(session_id.clone(), session);

        if has_history {
            info!(session = %session_id, "Restored session with history");
        }

        Ok(())
    }

    /// Initialize session store.
    pub async fn init(&self) -> Result<()> {
        self.session_store.init().await
    }
}
