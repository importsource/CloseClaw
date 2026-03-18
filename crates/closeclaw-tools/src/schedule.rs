use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;

use closeclaw_core::error::Result;
use closeclaw_core::schedule::ScheduleHandle;
use closeclaw_core::tool::{Tool, ToolDefinition, ToolResult};

/// Tool to add a new dynamic schedule.
pub struct AddScheduleTool {
    handle: Arc<dyn ScheduleHandle>,
}

impl AddScheduleTool {
    pub fn new(handle: Arc<dyn ScheduleHandle>) -> Self {
        Self { handle }
    }
}

#[async_trait]
impl Tool for AddScheduleTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "add_schedule".to_string(),
            description: "Add a new scheduled task that runs on a cron schedule. \
                The task will send the given message to the agent at each scheduled time. \
                Cron format: sec min hour dom month dow year (7 fields). \
                Examples: \"*/10 * * * * * *\" (every 10 seconds), \
                \"0 * * * * * *\" (every minute), \
                \"0 0 9 * * 1-5 *\" (weekdays at 9 AM)."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "Unique identifier for this schedule (e.g. 'encouragement', 'morning-news')"
                    },
                    "cron": {
                        "type": "string",
                        "description": "7-field cron expression: sec min hour dom month dow year"
                    },
                    "message": {
                        "type": "string",
                        "description": "The message to send to the agent at each scheduled time"
                    },
                    "agent_id": {
                        "type": "string",
                        "description": "Agent to handle the scheduled task (default: 'default')"
                    }
                },
                "required": ["id", "cron", "message"]
            }),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolResult> {
        let id = input["id"]
            .as_str()
            .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool("missing 'id' field".into()))?
            .to_string();
        let cron = input["cron"]
            .as_str()
            .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool("missing 'cron' field".into()))?
            .to_string();
        let message = input["message"]
            .as_str()
            .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool("missing 'message' field".into()))?
            .to_string();
        let agent_id = input["agent_id"]
            .as_str()
            .unwrap_or("default")
            .to_string();

        match self.handle.add_schedule(id, cron, agent_id, message).await {
            Ok(msg) => Ok(ToolResult::success(msg)),
            Err(e) => Ok(ToolResult::error(e)),
        }
    }
}

/// Tool to remove a dynamic schedule.
pub struct RemoveScheduleTool {
    handle: Arc<dyn ScheduleHandle>,
}

impl RemoveScheduleTool {
    pub fn new(handle: Arc<dyn ScheduleHandle>) -> Self {
        Self { handle }
    }
}

#[async_trait]
impl Tool for RemoveScheduleTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "remove_schedule".to_string(),
            description: "Remove a dynamically-created schedule by its ID. \
                Config-file schedules cannot be removed this way."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "id": {
                        "type": "string",
                        "description": "The ID of the schedule to remove"
                    }
                },
                "required": ["id"]
            }),
        }
    }

    async fn execute(&self, input: serde_json::Value) -> Result<ToolResult> {
        let id = input["id"]
            .as_str()
            .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool("missing 'id' field".into()))?;

        match self.handle.remove_schedule(id).await {
            Ok(msg) => Ok(ToolResult::success(msg)),
            Err(e) => Ok(ToolResult::error(e)),
        }
    }
}

/// Tool to list all active schedules.
pub struct ListSchedulesTool {
    handle: Arc<dyn ScheduleHandle>,
}

impl ListSchedulesTool {
    pub fn new(handle: Arc<dyn ScheduleHandle>) -> Self {
        Self { handle }
    }
}

#[async_trait]
impl Tool for ListSchedulesTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "list_schedules".to_string(),
            description: "List all active schedules (both config-file and dynamically-created).".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {}
            }),
        }
    }

    async fn execute(&self, _input: serde_json::Value) -> Result<ToolResult> {
        match self.handle.list_schedules().await {
            Ok(schedules) => {
                let json = serde_json::to_string_pretty(&schedules)
                    .unwrap_or_else(|_| "[]".to_string());
                Ok(ToolResult::success(json))
            }
            Err(e) => Ok(ToolResult::error(e)),
        }
    }
}
