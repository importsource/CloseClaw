use closeclaw_core::types::{
    ChannelId, Message, MessageContent, Sender, SessionId,
};
use closeclaw_gateway::hub::Hub;
use std::io::{self, BufRead, Write};
use std::sync::Arc;

/// Interactive CLI channel — reads from stdin, writes to stdout.
pub struct CliChannel {
    channel_id: ChannelId,
}

impl CliChannel {
    pub fn new() -> Self {
        Self {
            channel_id: ChannelId("cli".to_string()),
        }
    }

    /// Run the interactive REPL loop. This blocks until the user exits.
    pub async fn run(&self, hub: Arc<Hub>) {
        let session_id = SessionId(uuid::Uuid::new_v4().to_string());
        let user_id = "cli-user".to_string();
        let user_name = "User".to_string();

        println!("CloseClaw CLI — type your message, or 'quit' to exit.\n");

        let stdin = io::stdin();
        let mut stdout = io::stdout();

        loop {
            print!("> ");
            let _ = stdout.flush();

            let mut line = String::new();
            if stdin.lock().read_line(&mut line).unwrap_or(0) == 0 {
                break; // EOF
            }

            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed == "quit" || trimmed == "exit" {
                println!("Goodbye!");
                break;
            }

            let msg = Message {
                id: uuid::Uuid::new_v4().to_string(),
                session_id: session_id.clone(),
                channel_id: self.channel_id.clone(),
                sender: Sender::User {
                    name: user_name.clone(),
                    id: user_id.clone(),
                },
                content: MessageContent::Text(trimmed.to_string()),
                timestamp: chrono::Utc::now(),
            };

            match hub.handle_message(msg).await {
                Ok(response) => {
                    println!("\n{response}\n");
                }
                Err(e) => {
                    eprintln!("\nError: {e}\n");
                }
            }
        }
    }
}

impl Default for CliChannel {
    fn default() -> Self {
        Self::new()
    }
}
