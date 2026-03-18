use async_trait::async_trait;
use closeclaw_core::error::Result;
use closeclaw_core::tool::{Tool, ToolDefinition, ToolResult};
use serde_json::{json, Value};
use std::path::PathBuf;

pub struct ReadFileTool {
    workspace: PathBuf,
}

impl ReadFileTool {
    pub fn new(workspace: PathBuf) -> Self {
        Self { workspace }
    }

    fn resolve_path(&self, path: &str) -> PathBuf {
        let p = std::path::Path::new(path);
        if p.is_absolute() {
            p.to_path_buf()
        } else {
            self.workspace.join(path)
        }
    }
}

#[async_trait]
impl Tool for ReadFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "read_file".to_string(),
            description: "Read the contents of a file. Accepts absolute paths or paths relative to the workspace.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute file path, or relative to workspace"
                    },
                    "max_lines": {
                        "type": "integer",
                        "description": "Maximum number of lines to read (default: all)"
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

        let resolved = self.resolve_path(path);

        let content = match tokio::fs::read_to_string(&resolved).await {
            Ok(c) => c,
            Err(e) => return Ok(ToolResult::error(format!("Failed to read file: {e}"))),
        };

        let output = if let Some(max) = input["max_lines"].as_u64() {
            content
                .lines()
                .take(max as usize)
                .collect::<Vec<_>>()
                .join("\n")
        } else {
            content
        };

        Ok(ToolResult::success(output))
    }
}
