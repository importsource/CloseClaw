use closeclaw_core::skill::{Skill, SkillSource};
use closeclaw_core::types::ChatMessage;
use std::path::Path;

/// Assembles the system prompt from workspace files and skills.
pub struct ContextBuilder {
    parts: Vec<String>,
}

impl ContextBuilder {
    pub fn new() -> Self {
        Self { parts: Vec::new() }
    }

    /// Load a markdown file from the workspace if it exists, adding it to the system prompt.
    pub fn load_file(&mut self, workspace: &Path, filename: &str) -> &mut Self {
        let path = workspace.join(filename);
        if let Ok(content) = std::fs::read_to_string(&path) {
            self.parts
                .push(format!("# {filename}\n\n{content}"));
        }
        self
    }

    /// Load skills from a directory.
    pub fn load_skills(&mut self, dir: &Path) -> &mut Self {
        if !dir.is_dir() {
            return self;
        }
        let mut skills = Vec::new();
        if let Ok(entries) = std::fs::read_dir(dir) {
            for entry in entries.flatten() {
                let path = entry.path();
                if path.extension().map_or(false, |e| e == "md") {
                    if let Some(skill) = Skill::from_markdown(&path, SkillSource::Workspace) {
                        skills.push(skill);
                    }
                }
            }
        }
        if !skills.is_empty() {
            let mut section = String::from("# Available Skills\n\n");
            for skill in &skills {
                section.push_str(&format!("## {}\n{}\n\n{}\n\n", skill.name, skill.description, skill.content));
            }
            self.parts.push(section);
        }
        self
    }

    /// Add an explicit text section to the system prompt.
    pub fn add_section(&mut self, text: impl Into<String>) -> &mut Self {
        self.parts.push(text.into());
        self
    }

    /// Build the system prompt string.
    pub fn build(&self) -> String {
        self.parts.join("\n\n---\n\n")
    }

    /// Build a full message list with system prompt prepended.
    pub fn build_messages(&self, history: &[ChatMessage]) -> Vec<ChatMessage> {
        let mut messages = vec![ChatMessage::System(self.build())];
        messages.extend_from_slice(history);
        messages
    }
}

impl Default for ContextBuilder {
    fn default() -> Self {
        Self::new()
    }
}
