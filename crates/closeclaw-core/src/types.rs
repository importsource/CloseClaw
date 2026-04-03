use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::fmt;

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct AgentId(pub String);

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct SessionId(pub String);

#[derive(Debug, Clone, Hash, Eq, PartialEq, Serialize, Deserialize)]
pub struct ChannelId(pub String);

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for SessionId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl fmt::Display for ChannelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Sender {
    User { name: String, id: String },
    Agent { agent_id: AgentId },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MessageContent {
    Text(String),
    Image(Vec<u8>),
    File { name: String, bytes: Vec<u8> },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub id: String,
    pub session_id: SessionId,
    pub channel_id: ChannelId,
    pub sender: Sender,
    pub content: MessageContent,
    pub timestamp: DateTime<Utc>,
}

impl Message {
    pub fn user_text(
        session_id: SessionId,
        channel_id: ChannelId,
        user_name: &str,
        user_id: &str,
        text: impl Into<String>,
    ) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            session_id,
            channel_id,
            sender: Sender::User {
                name: user_name.to_string(),
                id: user_id.to_string(),
            },
            content: MessageContent::Text(text.into()),
            timestamp: Utc::now(),
        }
    }

    pub fn text_content(&self) -> Option<&str> {
        match &self.content {
            MessageContent::Text(s) => Some(s),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Event {
    MessageReceived(Message),
    AgentResponse {
        session_id: SessionId,
        content: String,
    },
    ToolInvoked {
        session_id: SessionId,
        tool: String,
        input: Value,
    },
    ToolResult {
        session_id: SessionId,
        tool: String,
        output: String,
        is_error: bool,
    },
    TextDelta {
        session_id: SessionId,
        text: String,
    },
    SessionCreated(SessionId),
    SessionReset(SessionId),
    Error {
        session_id: Option<SessionId>,
        error: String,
    },
    SystemNotice {
        message: String,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChatMessage {
    System(String),
    User(String),
    Assistant(String),
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
    ToolResult {
        id: String,
        output: String,
        is_error: bool,
    },
}
