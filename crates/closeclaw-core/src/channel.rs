use crate::error::Result;
use crate::types::{ChannelId, Event};
use async_trait::async_trait;
use tokio::sync::mpsc;

#[async_trait]
pub trait Channel: Send + Sync {
    fn id(&self) -> &ChannelId;
    async fn start(&self, event_tx: mpsc::Sender<Event>) -> Result<()>;
    async fn send_response(&self, session_id: &str, content: &str) -> Result<()>;
}
