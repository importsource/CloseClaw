use crate::context::ContextBuilder;
use crate::llm::{LlmProvider, LlmResponse};
use crate::tool_dispatch::ToolRegistry;
use async_trait::async_trait;
use closeclaw_core::agent::Agent;
use closeclaw_core::error::{CloseClawError, Result};
use closeclaw_core::session::Session;
use closeclaw_core::skill::{Skill, SkillSource};
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
    pub skills: Vec<Skill>,
}

impl AgentRuntime {
    pub fn new(
        llm: Arc<dyn LlmProvider>,
        tools: Arc<ToolRegistry>,
        workspace: PathBuf,
        skills_dirs: Vec<PathBuf>,
        max_iterations: usize,
    ) -> Self {
        let skills = load_all_skills(&skills_dirs);
        info!(count = skills.len(), "loaded skills at startup");

        Self {
            llm,
            tools,
            workspace,
            skills_dirs,
            max_iterations,
            skills,
        }
    }

    /// Build compact XML summary of all available skills (name + description only).
    /// Only includes skills where `disable_model_invocation == false`.
    fn inject_skill_summary(&self, ctx: &mut ContextBuilder) {
        let visible: Vec<&Skill> = self
            .skills
            .iter()
            .filter(|s| !s.disable_model_invocation)
            .collect();

        if visible.is_empty() {
            return;
        }

        let mut xml = String::from("<available-skills>\n");
        for skill in &visible {
            let slug = skill.slug();
            let emoji_attr = skill
                .metadata
                .get("emoji")
                .map(|e| format!(" emoji=\"{e}\""))
                .unwrap_or_default();

            let invocable_attr = if skill.user_invocable {
                format!(" slash=\"/{slug}\"")
            } else {
                String::new()
            };

            xml.push_str(&format!(
                "  <skill name=\"{}\"{invocable_attr}{emoji_attr}>{}</skill>\n",
                skill.name, skill.description,
            ));
        }
        xml.push_str("</available-skills>");

        ctx.add_section(xml);
    }

    /// Detect if user text starts with a `/slug` command.
    /// Returns `Some((skill, remaining_text))` if matched.
    fn detect_skill_invocation<'a>(&self, user_text: &'a str) -> Option<(&Skill, &'a str)> {
        let trimmed = user_text.trim();
        if !trimmed.starts_with('/') {
            return None;
        }

        for skill in &self.skills {
            if !skill.user_invocable {
                continue;
            }
            let slug = skill.slug();
            let command = format!("/{slug}");

            if trimmed == command || trimmed.starts_with(&format!("{command} ")) {
                let rest = trimmed
                    .strip_prefix(&command)
                    .unwrap_or("")
                    .trim_start();
                return Some((skill, rest));
            }
        }
        None
    }

    /// Build context messages, optionally injecting the full content of an activated skill.
    fn build_context(
        &self,
        session: &Session,
        activated_skill: Option<&Skill>,
    ) -> Vec<ChatMessage> {
        let mut ctx = ContextBuilder::new();

        // Inject current date/time so the LLM always knows "today"
        let now = chrono::Local::now();
        ctx.add_section(format!(
            "# Current Date and Time\n\nToday is {}. The current time is {}.",
            now.format("%A, %B %-d, %Y"),
            now.format("%-I:%M %p %Z"),
        ));

        ctx.load_file(&self.workspace, "SOUL.md")
            .load_file(&self.workspace, "AGENTS.md")
            .load_file(&self.workspace, "TOOLS.md")
            .load_file(&self.workspace, "IDENTITY.md")
            .load_file(&self.workspace, "USER.md");

        // Compact XML skill summary (always injected)
        self.inject_skill_summary(&mut ctx);

        // If a skill was activated via /slash command, inject its full content
        if let Some(skill) = activated_skill {
            ctx.add_section(format!(
                "# Active Skill: {}\n\n{}",
                skill.name, skill.content
            ));
        }

        ctx.build_messages(&session.history)
    }
}

// ---------------------------------------------------------------------------
// Skill loading
// ---------------------------------------------------------------------------

/// Load all skills from the given directories.
/// Later directories override earlier ones with the same slug (workspace > global).
fn load_all_skills(dirs: &[PathBuf]) -> Vec<Skill> {
    let mut skill_map: std::collections::HashMap<String, Skill> = std::collections::HashMap::new();

    for (idx, dir) in dirs.iter().enumerate() {
        if !dir.is_dir() {
            continue;
        }

        // Determine source based on directory order:
        // index 0 = global (~/.closeclaw/skills), rest = workspace/custom
        let source = if idx == 0 {
            SkillSource::Global
        } else {
            SkillSource::Workspace
        };

        let entries = match std::fs::read_dir(dir) {
            Ok(e) => e,
            Err(err) => {
                warn!(dir = %dir.display(), error = %err, "failed to read skills directory");
                continue;
            }
        };

        for entry in entries.flatten() {
            let path = entry.path();

            let skill = if path.is_dir() && path.join("SKILL.md").exists() {
                // New folder-based skill
                Skill::from_folder(&path, source.clone())
            } else if path.is_file()
                && path.extension().map_or(false, |e| e == "md")
            {
                // Legacy flat .md file
                Skill::from_legacy_markdown(&path, source.clone())
            } else {
                continue;
            };

            if let Some(skill) = skill {
                let slug = skill.slug();
                debug!(slug = %slug, name = %skill.name, "loaded skill");
                // Later entries (higher priority dirs) override earlier ones
                skill_map.insert(slug, skill);
            }
        }
    }

    skill_map.into_values().collect()
}

// ---------------------------------------------------------------------------
// Agent trait impl
// ---------------------------------------------------------------------------

#[async_trait]
impl Agent for AgentRuntime {
    async fn handle_message(
        &self,
        session: &mut Session,
        user_text: &str,
        event_tx: &mpsc::Sender<Event>,
    ) -> Result<String> {
        // Detect /slash command invocation
        let (activated_skill, effective_text) =
            if let Some((skill, rest)) = self.detect_skill_invocation(user_text) {
                info!(skill = %skill.name, slug = %skill.slug(), "skill invoked via slash command");
                let text = if rest.is_empty() {
                    user_text.to_string()
                } else {
                    rest.to_string()
                };
                (Some(skill), text)
            } else {
                (None, user_text.to_string())
            };

        // Add user message to session history
        session.append(ChatMessage::User(effective_text));

        let tool_defs = self.tools.definitions();

        for iteration in 0..self.max_iterations {
            let messages = self.build_context(session, activated_skill);

            debug!("ReAct iteration {iteration}, history len = {}", messages.len());

            // Create a delta channel and forward text deltas as events
            let (delta_tx, mut delta_rx) = mpsc::channel::<String>(256);
            let delta_session_id = session.id.clone();
            let delta_event_tx = event_tx.clone();
            let delta_forwarder = tokio::spawn(async move {
                while let Some(text) = delta_rx.recv().await {
                    let _ = delta_event_tx
                        .send(Event::TextDelta {
                            session_id: delta_session_id.clone(),
                            text,
                        })
                        .await;
                }
            });

            let response = self.llm.chat_stream(&messages, &tool_defs, &delta_tx).await;
            drop(delta_tx); // close channel so forwarder stops
            let _ = delta_forwarder.await;
            let response = response?;

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
