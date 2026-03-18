use closeclaw_core::types::{ChannelId, Message, MessageContent, Sender};
use closeclaw_gateway::hub::Hub;
use std::sync::Arc;
use teloxide::prelude::*;
use tracing::{error, info};

/// Maximum message length allowed by the Telegram Bot API.
const TELEGRAM_MAX_LEN: usize = 4096;

/// Telegram bot channel — receives messages via long polling, sends responses back.
pub struct TelegramChannel {
    token: String,
}

impl TelegramChannel {
    pub fn new(token: String) -> Self {
        Self { token }
    }

    /// Run the Telegram bot with long polling. This blocks until the bot is stopped.
    pub async fn run(self, hub: Arc<Hub>) {
        let bot = Bot::new(&self.token);

        info!("Telegram channel starting (long polling)");

        let channel_id = ChannelId("telegram".to_string());

        teloxide::repl(bot, move |bot: Bot, telegram_msg: teloxide::types::Message| {
            let hub = hub.clone();
            let channel_id = channel_id.clone();
            async move {
                let text = match telegram_msg.text() {
                    Some(t) => t.to_string(),
                    None => return Ok(()), // ignore non-text messages
                };

                let chat_id = telegram_msg.chat.id;
                let peer_id = format!("tg:{}", chat_id.0);
                let user_name = telegram_msg
                    .from
                    .as_ref()
                    .and_then(|u| u.username.clone())
                    .unwrap_or_else(|| "TelegramUser".to_string());

                // Send typing indicator while the agent processes
                let _ = bot
                    .send_chat_action(chat_id, teloxide::types::ChatAction::Typing)
                    .await;

                let msg = Message {
                    id: uuid::Uuid::new_v4().to_string(),
                    session_id: closeclaw_core::types::SessionId(String::new()), // Hub router will assign
                    channel_id,
                    sender: Sender::User {
                        name: user_name,
                        id: peer_id,
                    },
                    content: MessageContent::Text(text),
                    timestamp: chrono::Utc::now(),
                };

                let response = match hub.handle_message(msg).await {
                    Ok(r) => r,
                    Err(e) => {
                        error!("Agent error for chat {chat_id}: {e}");
                        format!("Error: {e}")
                    }
                };

                for chunk in split_message(&response) {
                    if let Err(e) = bot.send_message(chat_id, chunk).await {
                        error!("Failed to send Telegram message to {chat_id}: {e}");
                        break;
                    }
                }

                Ok(())
            }
        })
        .await;
    }
}

/// Split a message into chunks that fit within Telegram's 4096-character limit.
/// Splits at newline boundaries when possible, otherwise hard-splits.
pub fn split_message(text: &str) -> Vec<&str> {
    if text.len() <= TELEGRAM_MAX_LEN {
        return vec![text];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= TELEGRAM_MAX_LEN {
            chunks.push(remaining);
            break;
        }

        // Try to find a newline boundary within the limit
        let split_at = match remaining[..TELEGRAM_MAX_LEN].rfind('\n') {
            Some(pos) if pos > 0 => pos + 1, // include the newline in current chunk
            _ => TELEGRAM_MAX_LEN,            // hard split
        };

        chunks.push(&remaining[..split_at]);
        remaining = &remaining[split_at..];
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_short_message() {
        let msg = "Hello, world!";
        let chunks = split_message(msg);
        assert_eq!(chunks, vec!["Hello, world!"]);
    }

    #[test]
    fn test_split_at_newline_boundary() {
        // Create a message where the first ~4000 chars end with some lines,
        // then more text follows
        let line = "x".repeat(100);
        let mut msg = String::new();
        for _ in 0..50 {
            msg.push_str(&line);
            msg.push('\n');
        }
        // msg is now 50 * 101 = 5050 chars, exceeds 4096

        let chunks = split_message(&msg);
        assert!(chunks.len() >= 2);
        // First chunk should end at a newline and be <= 4096
        assert!(chunks[0].len() <= TELEGRAM_MAX_LEN);
        assert!(chunks[0].ends_with('\n'));
        // All chunks together should equal the original
        let reassembled: String = chunks.concat();
        assert_eq!(reassembled, msg);
    }

    #[test]
    fn test_split_no_newlines() {
        let msg = "a".repeat(5000);
        let chunks = split_message(&msg);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), TELEGRAM_MAX_LEN);
        assert_eq!(chunks[1].len(), 5000 - TELEGRAM_MAX_LEN);
    }

    #[test]
    fn test_split_empty() {
        let chunks = split_message("");
        assert_eq!(chunks, vec![""]);
    }
}
