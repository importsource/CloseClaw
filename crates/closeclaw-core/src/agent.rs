use crate::error::Result;
use crate::session::Session;
use crate::types::Event;
use async_trait::async_trait;
use tokio::sync::mpsc;

#[async_trait]
pub trait Agent: Send + Sync {
    async fn handle_message(
        &self,
        session: &mut Session,
        user_text: &str,
        event_tx: &mpsc::Sender<Event>,
    ) -> Result<String>;
}
