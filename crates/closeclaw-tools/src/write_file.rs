use async_trait::async_trait;
use closeclaw_core::error::Result;
use closeclaw_core::tool::{Tool, ToolDefinition, ToolResult};
use serde_json::{json, Value};
use std::path::PathBuf;

pub struct WriteFileTool {
    workspace: PathBuf,
}

impl WriteFileTool {
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
impl Tool for WriteFileTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "write_file".to_string(),
            description: "Write content to a file. Creates parent directories if needed. Accepts absolute paths or paths relative to the workspace.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "path": {
                        "type": "string",
                        "description": "Absolute file path, or relative to workspace"
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

        let resolved = self.resolve_path(path);

        // Create parent directories
        if let Some(parent) = resolved.parent() {
            if let Err(e) = tokio::fs::create_dir_all(parent).await {
                return Ok(ToolResult::error(format!(
                    "Failed to create directories: {e}"
                )));
            }
        }

        match tokio::fs::write(&resolved, content).await {
            Ok(()) => Ok(ToolResult::success(format!(
                "Written {} bytes to {}",
                content.len(),
                resolved.display()
            ))),
            Err(e) => Ok(ToolResult::error(format!("Failed to write file: {e}"))),
        }
    }
}
