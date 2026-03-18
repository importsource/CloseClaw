use thiserror::Error;

#[derive(Debug, Error)]
pub enum CloseClawError {
    #[error("Tool error: {0}")]
    Tool(String),

    #[error("LLM error: {0}")]
    Llm(String),

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Agent not found: {0}")]
    AgentNotFound(String),

    #[error("Channel error: {0}")]
    Channel(String),

    #[error("Config error: {0}")]
    Config(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),

    #[error("JSON error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("Max iterations reached ({0})")]
    MaxIterations(usize),

    #[error("{0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, CloseClawError>;
