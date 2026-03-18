use async_trait::async_trait;
use closeclaw_core::error::Result;
use closeclaw_core::tool::{Tool, ToolDefinition, ToolResult};
use serde_json::{json, Value};
use std::path::PathBuf;

fn resolve_path(workspace: &std::path::Path, path: &str) -> PathBuf {
    let p = std::path::Path::new(path);
    if p.is_absolute() {
        p.to_path_buf()
    } else {
        workspace.join(path)
    }
}

// ---------- list_files ----------

pub struct ListFilesTool {
    workspace: PathBuf,
}

impl ListFilesTool {
    pub fn new(workspace: PathBuf) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for ListFilesTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_files".to_string(),
            description: "List files and directories. Accepts absolute paths or paths relative to the workspace.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Directory path (absolute or relative to workspace). Defaults to workspace root."
                    }
                },
                "required": []
            }),
        }
    }

    async fn execute(&self, input: Value) -> Result<ToolResult> {
        let path = input["path"].as_str().unwrap_or(".");
        let resolved = resolve_path(&self.workspace, path);

        if !resolved.is_dir() {
            return Ok(ToolResult::error(format!("{} is not a directory", resolved.display())));
        }

        let entries = match std::fs::read_dir(&resolved) {
            Ok(dir) => dir,
            Err(e) => return Ok(ToolResult::error(format!("Failed to list directory: {e}"))),
        };

        let mut lines = Vec::new();
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            let meta = entry.metadata();
            let (kind, size) = match meta {
                Ok(m) => {
                    let k = if m.is_dir() { "dir " } else { "file" };
                    (k, m.len())
                }
                Err(_) => ("????", 0),
            };
            lines.push(format!("{kind}\t{size}\t{name}"));
        }

        if lines.is_empty() {
            Ok(ToolResult::success("(empty directory)"))
        } else {
            Ok(ToolResult::success(lines.join("\n")))
        }
    }
}

// ---------- create_file ----------

pub struct CreateFileTool {
    workspace: PathBuf,
}

impl CreateFileTool {
    pub fn new(workspace: PathBuf) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for CreateFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "create_file".to_string(),
            description: "Create or overwrite a file. Parent directories are created automatically. Accepts absolute paths or paths relative to the workspace.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "File path (absolute or relative to workspace)"
                    },
                    "content": {
                        "type": "string",
                        "description": "Content to write to the file"
                    }
                },
                "required": ["path", "content"]
            }),
        }
    }

    async fn execute(&self, input: Value) -> Result<ToolResult> {
        let path = input["path"]
            .as_str()
            .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool("missing 'path' field".into()))?;
        let content = input["content"]
            .as_str()
            .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool("missing 'content' field".into()))?;

        let resolved = resolve_path(&self.workspace, path);

        // Create parent directories
        if let Some(parent) = resolved.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return Ok(ToolResult::error(format!("Failed to create directories: {e}")));
            }
        }

        match tokio::fs::write(&resolved, content).await {
            Ok(()) => Ok(ToolResult::success(format!(
                "Created {} ({} bytes)",
                resolved.display(),
                content.len()
            ))),
            Err(e) => Ok(ToolResult::error(format!("Failed to create file: {e}"))),
        }
    }
}

// ---------- delete_file ----------

pub struct DeleteFileTool {
    workspace: PathBuf,
}

impl DeleteFileTool {
    pub fn new(workspace: PathBuf) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for DeleteFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "delete_file".to_string(),
            description: "Delete a file or directory. Accepts absolute paths or paths relative to the workspace.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Path to delete (absolute or relative to workspace)"
                    }
                },
                "required": ["path"]
            }),
        }
    }

    async fn execute(&self, input: Value) -> Result<ToolResult> {
        let path = input["path"]
            .as_str()
            .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool("missing 'path' field".into()))?;

        let resolved = resolve_path(&self.workspace, path);

        if !resolved.exists() {
            return Ok(ToolResult::error(format!("{} does not exist", resolved.display())));
        }

        let result = if resolved.is_dir() {
            tokio::fs::remove_dir_all(&resolved).await
        } else {
            tokio::fs::remove_file(&resolved).await
        };

        match result {
            Ok(()) => Ok(ToolResult::success(format!("Deleted {}", resolved.display()))),
            Err(e) => Ok(ToolResult::error(format!("Failed to delete: {e}"))),
        }
    }
}

// ---------- search_files ----------

pub struct SearchFilesTool {
    workspace: PathBuf,
}

impl SearchFilesTool {
    pub fn new(workspace: PathBuf) -> Self {
        Self { workspace }
    }
}

#[async_trait]
impl Tool for SearchFilesTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "search_files".to_string(),
            description: "Search for files by name pattern or content. Accepts absolute paths or paths relative to the workspace.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "name": {
                        "type": "string",
                        "description": "Filename pattern to match (substring, case-insensitive). E.g. '.txt' matches all text files."
                    },
                    "content": {
                        "type": "string",
                        "description": "Search for files containing this text (case-insensitive substring match)"
                    },
                    "path": {
                        "type": "string",
                        "description": "Directory to search in (absolute or relative to workspace). Defaults to workspace root."
                    }
                },
                "required": []
            }),
        }
    }

    async fn execute(&self, input: Value) -> Result<ToolResult> {
        let name_pattern = input["name"].as_str().map(|s| s.to_lowercase());
        let content_pattern = input["content"].as_str().map(|s| s.to_lowercase());
        let sub_path = input["path"].as_str().unwrap_or(".");

        let resolved = resolve_path(&self.workspace, sub_path);

        if !resolved.is_dir() {
            return Ok(ToolResult::error(format!("{} is not a directory", resolved.display())));
        }

        let mut matches = Vec::new();
        let mut stack = vec![resolved];

        while let Some(dir) = stack.pop() {
            let entries = match std::fs::read_dir(&dir) {
                Ok(e) => e,
                Err(_) => continue,
            };
            for entry in entries.flatten() {
                let path = entry.path();
                if path.is_dir() {
                    stack.push(path);
                    continue;
                }

                let file_name = entry.file_name().to_string_lossy().to_string();

                // Filter by name pattern
                if let Some(ref pattern) = name_pattern {
                    if !file_name.to_lowercase().contains(pattern) {
                        continue;
                    }
                }

                let display_path = path.display().to_string();

                // Filter by content
                if let Some(ref pattern) = content_pattern {
                    if let Ok(text) = std::fs::read_to_string(&path) {
                        if text.to_lowercase().contains(pattern) {
                            let mut matched_lines = Vec::new();
                            for (i, line) in text.lines().enumerate() {
                                if line.to_lowercase().contains(pattern) {
                                    matched_lines.push(format!("  {}:{}: {}", display_path, i + 1, line));
                                }
                            }
                            matches.push(matched_lines.join("\n"));
                        }
                    }
                } else {
                    matches.push(display_path);
                }

                if matches.len() >= 100 {
                    matches.push("... (truncated at 100 results)".to_string());
                    break;
                }
            }
        }

        if matches.is_empty() {
            Ok(ToolResult::success("No matches found."))
        } else {
            Ok(ToolResult::success(matches.join("\n")))
        }
    }
}
