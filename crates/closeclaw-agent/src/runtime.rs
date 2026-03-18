use crate::context::ContextBuilder;
use crate::llm::{LlmProvider, LlmResponse};
use crate::tool_dispatch::ToolRegistry;
use async_trait::async_trait;
use closeclaw_core::agent::Agent;
use closeclaw_core::error::{CloseClawError, Result};
use closeclaw_core::session::Session;
use closeclaw_core::types::{ChatMessage, Event};
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

pub struct AgentRuntime {
    pub llm: Arc<dyn LlmProvider>,
    pub tools: Arc<ToolRegistry>,
    pub workspace: PathBuf,
    pub skills_dirs: Vec<PathBuf>,
    pub max_iterations: usize,
}

impl AgentRuntime {
    pub fn new(
        llm: Arc<dyn LlmProvider>,
        tools: Arc<ToolRegistry>,
        workspace: PathBuf,
        skills_dirs: Vec<PathBuf>,
        max_iterations: usize,
    ) -> Self {
        Self {
            llm,
            tools,
            workspace,
            skills_dirs,
            max_iterations,
        }
    }

    fn build_context(&self, session: &Session) -> Vec<ChatMessage> {
        let mut ctx = ContextBuilder::new();
        ctx.load_file(&self.workspace, "SOUL.md")
            .load_file(&self.workspace, "AGENTS.md")
            .load_file(&self.workspace, "TOOLS.md")
            .load_file(&self.workspace, "IDENTITY.md")
            .load_file(&self.workspace, "USER.md");

        for dir in &self.skills_dirs {
            ctx.load_skills(dir);
        }

        ctx.build_messages(&session.history)
    }
}

#[async_trait]
impl Agent for AgentRuntime {
    async fn handle_message(
        &self,
        session: &mut Session,
        user_text: &str,
        event_tx: &mpsc::Sender<Event>,
    ) -> Result<String> {
        // Add user message to session history
        session.append(ChatMessage::User(user_text.to_string()));

        let tool_defs = self.tools.definitions();

        for iteration in 0..self.max_iterations {
            let messages = self.build_context(session);

            debug!("ReAct iteration {iteration}, history len = {}", messages.len());

            let response = self.llm.chat(&messages, &tool_defs).await?;

            match response {
                LlmResponse::Text(text) => {
                    info!("Agent responded with text (iter {iteration})");
                    session.append(ChatMessage::Assistant(text.clone()));
                    return Ok(text);
                }
                LlmResponse::ToolUse(calls) => {
                    for call in calls {
                        info!("Tool call: {} (iter {iteration})", call.name);

                        // Emit tool invoked event
                        let _ = event_tx
                            .send(Event::ToolInvoked {
                                session_id: session.id.clone(),
                                tool: call.name.clone(),
                                input: call.input.clone(),
                            })
                            .await;

                        // Append tool use to history
                        session.append(ChatMessage::ToolUse {
                            id: call.id.clone(),
                            name: call.name.clone(),
                            input: call.input.clone(),
                        });

                        // Execute tool
                        let result = self.tools.dispatch(&call.name, call.input).await?;

                        // Emit tool result event
                        let _ = event_tx
                            .send(Event::ToolResult {
                                session_id: session.id.clone(),
                                tool: call.name.clone(),
                                output: result.output.clone(),
                                is_error: result.is_error,
                            })
                            .await;

                        // Append result to history
                        session.append(ChatMessage::ToolResult {
                            id: call.id,
                            output: result.output,
                            is_error: result.is_error,
                        });
                    }
                }
            }
        }

        warn!("Max iterations ({}) reached", self.max_iterations);
        Err(CloseClawError::MaxIterations(self.max_iterations))
    }
}
