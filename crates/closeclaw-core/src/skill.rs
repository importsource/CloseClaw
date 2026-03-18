use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SkillSource {
    Workspace,
    Local,
    Bundled,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
    pub source: SkillSource,
}

impl Skill {
    /// Parse a SKILL.md file into a Skill struct.
    /// Expected format:
    /// ```markdown
    /// # Skill Name
    /// Description line
    ///
    /// Body content...
    /// ```
    pub fn from_markdown(path: &Path, source: SkillSource) -> Option<Self> {
        let content = std::fs::read_to_string(path).ok()?;
        let mut lines = content.lines();

        let name = lines
            .next()?
            .trim_start_matches('#')
            .trim()
            .to_string();

        let description = lines
            .next()
            .unwrap_or("")
            .trim()
            .to_string();

        Some(Self {
            name,
            description,
            content,
            source,
        })
    }
}
