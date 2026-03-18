use closeclaw_core::types::{AgentId, ChannelId, SessionId};
use dashmap::DashMap;

/// Routing key: (channel_id, peer_id) → determines session + agent
#[derive(Debug, Clone, Hash, Eq, PartialEq)]
pub struct RouteKey {
    pub channel_id: ChannelId,
    pub peer_id: String,
}

pub struct Router {
    /// Mapping of route key → (agent_id, session_id)
    routes: DashMap<RouteKey, (AgentId, SessionId)>,
    /// Default agent ID (first agent in config)
    default_agent: AgentId,
}

impl Router {
    pub fn new(default_agent: AgentId) -> Self {
        Self {
            routes: DashMap::new(),
            default_agent,
        }
    }

    /// Resolve which agent and session should handle a message from a given channel+peer.
    /// Creates a new session mapping if none exists.
    pub fn resolve(
        &self,
        channel_id: &ChannelId,
        peer_id: &str,
    ) -> (AgentId, SessionId, bool) {
        let key = RouteKey {
            channel_id: channel_id.clone(),
            peer_id: peer_id.to_string(),
        };

        if let Some(entry) = self.routes.get(&key) {
            let (agent_id, session_id) = entry.value().clone();
            return (agent_id, session_id, false);
        }

        let session_id = SessionId(uuid::Uuid::new_v4().to_string());
        let agent_id = self.default_agent.clone();
        self.routes
            .insert(key, (agent_id.clone(), session_id.clone()));
        (agent_id, session_id, true)
    }

    /// Pre-register a route with a known session ID (e.g., for scheduler sessions
    /// that need deterministic IDs across restarts).
    pub fn seed(
        &self,
        channel_id: ChannelId,
        peer_id: String,
        agent_id: AgentId,
        session_id: SessionId,
    ) {
        let key = RouteKey {
            channel_id,
            peer_id,
        };
        self.routes.insert(key, (agent_id, session_id));
    }

    /// Remove a route (e.g., on session reset).
    pub fn remove(&self, channel_id: &ChannelId, peer_id: &str) {
        let key = RouteKey {
            channel_id: channel_id.clone(),
            peer_id: peer_id.to_string(),
        };
        self.routes.remove(&key);
    }
}
