use closeclaw_core::error::{CloseClawError, Result};
use closeclaw_core::types::{ChatMessage, SessionId};
use std::path::PathBuf;

/// File-based session persistence using JSONL format.
pub struct SessionStore {
    dir: PathBuf,
}

impl SessionStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn session_path(&self, id: &SessionId) -> PathBuf {
        self.dir.join(format!("{}.jsonl", id.0))
    }

    /// Ensure the storage directory exists.
    pub async fn init(&self) -> Result<()> {
        tokio::fs::create_dir_all(&self.dir).await?;
        Ok(())
    }

    /// Append a chat message to the session's JSONL transcript.
    pub async fn append_message(&self, session_id: &SessionId, msg: &ChatMessage) -> Result<()> {
        let path = self.session_path(session_id);
        let line = serde_json::to_string(msg)? + "\n";
        tokio::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&path)
            .await?
            .write_all(line.as_bytes())
            .await
            .map_err(|e| CloseClawError::Io(e))?;
        Ok(())
    }

    /// Load session history from a JSONL file.
    pub async fn load_history(&self, session_id: &SessionId) -> Result<Vec<ChatMessage>> {
        let path = self.session_path(session_id);
        if !path.exists() {
            return Ok(Vec::new());
        }
        let content = tokio::fs::read_to_string(&path).await?;
        let mut messages = Vec::new();
        for line in content.lines() {
            if line.trim().is_empty() {
                continue;
            }
            let msg: ChatMessage = serde_json::from_str(line)?;
            messages.push(msg);
        }
        Ok(messages)
    }

    /// Delete a session's transcript file.
    pub async fn delete(&self, session_id: &SessionId) -> Result<()> {
        let path = self.session_path(session_id);
        if path.exists() {
            tokio::fs::remove_file(&path).await?;
        }
        Ok(())
    }
}

use tokio::io::AsyncWriteExt;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_append_and_load() {
        let dir = std::env::temp_dir().join(format!("closeclaw_test_{}", uuid::Uuid::new_v4()));
        let store = SessionStore::new(dir.clone());
        store.init().await.unwrap();

        let sid = SessionId("test-session".to_string());
        store
            .append_message(&sid, &ChatMessage::User("hello".to_string()))
            .await
            .unwrap();
        store
            .append_message(&sid, &ChatMessage::Assistant("hi there".to_string()))
            .await
            .unwrap();

        let history = store.load_history(&sid).await.unwrap();
        assert_eq!(history.len(), 2);
        match &history[0] {
            ChatMessage::User(text) => assert_eq!(text, "hello"),
            other => panic!("Expected User, got {:?}", other),
        }
        match &history[1] {
            ChatMessage::Assistant(text) => assert_eq!(text, "hi there"),
            other => panic!("Expected Assistant, got {:?}", other),
        }

        // Cleanup
        store.delete(&sid).await.unwrap();
        let _ = tokio::fs::remove_dir_all(&dir).await;
    }

    #[tokio::test]
    async fn test_load_nonexistent_session() {
        let dir = std::env::temp_dir().join(format!("closeclaw_test_{}", uuid::Uuid::new_v4()));
        let store = SessionStore::new(dir.clone());
        store.init().await.unwrap();

        let sid = SessionId("nonexistent".to_string());
        let history = store.load_history(&sid).await.unwrap();
        assert!(history.is_empty());

        let _ = tokio::fs::remove_dir_all(&dir).await;
    }
}
