use crate::types::{AgentId, ChannelId, ChatMessage, SessionId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Session {
    pub id: SessionId,
    pub agent_id: AgentId,
    pub channel_id: ChannelId,
    pub history: Vec<ChatMessage>,
    pub created_at: DateTime<Utc>,
    pub last_active: DateTime<Utc>,
    pub metadata: HashMap<String, Value>,
}

impl Session {
    pub fn new(id: SessionId, agent_id: AgentId, channel_id: ChannelId) -> Self {
        let now = Utc::now();
        Self {
            id,
            agent_id,
            channel_id,
            history: Vec::new(),
            created_at: now,
            last_active: now,
            metadata: HashMap::new(),
        }
    }

    pub fn append(&mut self, msg: ChatMessage) {
        self.last_active = Utc::now();
        self.history.push(msg);
    }
}
