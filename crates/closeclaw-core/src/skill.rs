use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use tracing::warn;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SkillSource {
    Workspace,
    Global,
    Bundled,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SkillRequires {
    #[serde(default)]
    pub bins: Vec<String>,
    #[serde(default)]
    pub env: Vec<String>,
    #[serde(default)]
    pub os: Vec<String>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SkillMetadata {
    #[serde(default)]
    pub emoji: Option<String>,
    #[serde(default)]
    pub requires: Option<SkillRequires>,
}

#[derive(Debug, Clone, Default, Deserialize)]
pub struct SkillFrontmatter {
    pub name: Option<String>,
    pub description: Option<String>,
    #[serde(default, rename = "user-invocable")]
    pub user_invocable: Option<bool>,
    #[serde(default, rename = "disable-model-invocation")]
    pub disable_model_invocation: Option<bool>,
    #[serde(default)]
    pub metadata: Option<SkillMetadata>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Skill {
    pub name: String,
    pub description: String,
    pub content: String,
    pub source: SkillSource,
    pub user_invocable: bool,
    pub disable_model_invocation: bool,
    pub metadata: HashMap<String, String>,
    pub path: PathBuf,
}

// ---------------------------------------------------------------------------
// Parsing helpers
// ---------------------------------------------------------------------------

/// Split a raw string at the YAML frontmatter delimiters (`---`).
/// Returns `(yaml_str, body_str)`. If no frontmatter found, returns `(None, full_text)`.
fn parse_frontmatter(raw: &str) -> (Option<&str>, &str) {
    let trimmed = raw.trim_start();
    if !trimmed.starts_with("---") {
        return (None, raw);
    }
    // Skip the leading `---\n`
    let after_open = &trimmed[3..];
    let after_open = after_open.strip_prefix('\n').unwrap_or(after_open);

    if let Some(close_pos) = after_open.find("\n---") {
        let yaml = &after_open[..close_pos];
        let body_start = close_pos + 4; // skip `\n---`
        let body = if body_start < after_open.len() {
            after_open[body_start..].trim_start_matches('\n')
        } else {
            ""
        };
        (Some(yaml), body)
    } else {
        (None, raw)
    }
}

/// Check gating requirements. Returns `true` if the skill should be loaded.
fn check_gating(meta: &SkillMetadata) -> bool {
    if let Some(ref requires) = meta.requires {
        // Check required binaries
        for bin in &requires.bins {
            if which::which(bin).is_err() {
                warn!(skill_bin = %bin, "skill gating: required binary not found, skipping skill");
                return false;
            }
        }
        // Check required environment variables
        for var in &requires.env {
            if std::env::var(var).is_err() {
                warn!(skill_env = %var, "skill gating: required env var not set, skipping skill");
                return false;
            }
        }
        // Check OS
        if !requires.os.is_empty() {
            let current_os = std::env::consts::OS;
            if !requires.os.iter().any(|os| os == current_os) {
                warn!(skill_os = %current_os, required = ?requires.os, "skill gating: OS mismatch, skipping skill");
                return false;
            }
        }
    }
    true
}

// ---------------------------------------------------------------------------
// Skill impl
// ---------------------------------------------------------------------------

impl Skill {
    /// Derive a URL-safe slug from the skill path.
    /// For folder-based skills (`skills/code-review/SKILL.md`), uses the folder name.
    /// For legacy flat files (`skills/code_review.md`), uses the file stem with `_` → `-`.
    pub fn slug(&self) -> String {
        if self.path.file_name().map_or(false, |f| f == "SKILL.md") {
            // Folder-based: use parent directory name
            self.path
                .parent()
                .and_then(|p| p.file_name())
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_lowercase()
                .replace('_', "-")
        } else {
            // Legacy flat file: use file stem
            self.path
                .file_stem()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_lowercase()
                .replace('_', "-")
        }
    }

    /// Load a skill from a folder containing `SKILL.md` with YAML frontmatter.
    pub fn from_folder(dir: &Path, source: SkillSource) -> Option<Self> {
        let skill_path = dir.join("SKILL.md");
        let raw = std::fs::read_to_string(&skill_path).ok()?;

        let (yaml_str, body) = parse_frontmatter(&raw);

        let fm: SkillFrontmatter = if let Some(yaml) = yaml_str {
            serde_yml::from_str(yaml).unwrap_or_else(|e| {
                warn!(path = %skill_path.display(), error = %e, "failed to parse YAML frontmatter");
                SkillFrontmatter::default()
            })
        } else {
            SkillFrontmatter::default()
        };

        let metadata_obj = fm.metadata.clone().unwrap_or_default();

        // Run gating checks
        if !check_gating(&metadata_obj) {
            return None;
        }

        // Fall back to folder name for name/description if not in frontmatter
        let folder_name = dir
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("unknown");

        let name = fm.name.unwrap_or_else(|| {
            folder_name
                .split('-')
                .map(|w| {
                    let mut c = w.chars();
                    match c.next() {
                        Some(first) => {
                            first.to_uppercase().to_string() + c.as_str()
                        }
                        None => String::new(),
                    }
                })
                .collect::<Vec<_>>()
                .join(" ")
        });

        let description = fm.description.unwrap_or_default();

        // Build flat metadata map for XML attributes
        let mut meta_map = HashMap::new();
        if let Some(ref emoji) = metadata_obj.emoji {
            meta_map.insert("emoji".to_string(), emoji.clone());
        }

        Some(Self {
            name,
            description,
            content: body.to_string(),
            source,
            user_invocable: fm.user_invocable.unwrap_or(true),
            disable_model_invocation: fm.disable_model_invocation.unwrap_or(false),
            metadata: meta_map,
            path: skill_path,
        })
    }

    /// Parse a flat `.md` file (legacy format) into a Skill.
    /// Expected format:
    /// ```markdown
    /// # Skill Name
    /// Description line
    ///
    /// Body content...
    /// ```
    pub fn from_legacy_markdown(path: &Path, source: SkillSource) -> Option<Self> {
        let raw = std::fs::read_to_string(path).ok()?;
        let mut lines = raw.lines();

        let name = lines
            .next()?
            .trim_start_matches('#')
            .trim()
            .to_string();

        let description = lines.next().unwrap_or("").trim().to_string();

        Some(Self {
            name,
            description,
            content: raw.clone(),
            source,
            user_invocable: true,
            disable_model_invocation: false,
            metadata: HashMap::new(),
            path: path.to_path_buf(),
        })
    }
}
