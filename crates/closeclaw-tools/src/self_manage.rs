use async_trait::async_trait;
use closeclaw_core::error::Result;
use closeclaw_core::tool::{Tool, ToolDefinition, ToolResult};
use serde_json::{json, Value};
use std::path::PathBuf;

/// Well-known marker file path indicating a restart is in progress.
/// On startup, if this file exists, the server knows it just restarted.
pub fn restart_marker_path() -> PathBuf {
    let home = std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."));
    home.join(".closeclaw/restart-pending")
}

/// Tool that allows the agent to manage its own process (e.g. restart).
pub struct SelfManageTool;

#[async_trait]
impl Tool for SelfManageTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "self_manage".to_string(),
            description: "Manage the CloseClaw server process. Supports restarting the server."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "enum": ["restart"],
                        "description": "The action to perform. Currently only 'restart' is supported."
                    }
                },
                "required": ["action"]
            }),
        }
    }

    async fn execute(&self, input: Value) -> Result<ToolResult> {
        let action = input["action"].as_str().ok_or_else(|| {
            closeclaw_core::error::CloseClawError::Tool("missing 'action' field".into())
        })?;

        match action {
            "restart" => {
                tokio::spawn(async {
                    // 5 seconds gives the LLM enough time to generate and stream
                    // a response to the user before the process is replaced.
                    tokio::time::sleep(std::time::Duration::from_secs(5)).await;
                    restart_process();
                });
                Ok(ToolResult::success(
                    "Restarting in 5 seconds. Tell the user the server is about to restart.",
                ))
            }
            other => Ok(ToolResult::error(format!("Unknown action: {other}"))),
        }
    }
}

/// Write marker file, then replace the current process with a fresh instance.
fn restart_process() {
    use std::os::unix::process::CommandExt;

    // Write marker so the new process knows it was a restart
    let marker = restart_marker_path();
    if let Some(parent) = marker.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let _ = std::fs::write(&marker, "restart");

    let exe = std::env::current_exe().expect("failed to get current executable path");
    let args: Vec<String> = std::env::args().skip(1).collect();

    let err = std::process::Command::new(&exe).args(&args).exec();
    // exec() only returns on error
    eprintln!("Failed to restart: {err}");
    std::process::exit(1);
}
