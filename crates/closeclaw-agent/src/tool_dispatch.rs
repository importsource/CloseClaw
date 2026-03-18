use closeclaw_core::error::{CloseClawError, Result};
use closeclaw_core::tool::{Tool, ToolDefinition, ToolResult};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;

pub struct ToolRegistry {
    tools: HashMap<String, Arc<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Arc<dyn Tool>) {
        let name = tool.definition().name;
        self.tools.insert(name, tool);
    }

    pub fn definitions(&self) -> Vec<ToolDefinition> {
        self.tools.values().map(|t| t.definition()).collect()
    }

    pub async fn dispatch(&self, name: &str, input: Value) -> Result<ToolResult> {
        let tool = self
            .tools
            .get(name)
            .ok_or_else(|| CloseClawError::Tool(format!("Unknown tool: {name}")))?;

        tool.execute(input).await
    }

    pub fn has(&self, name: &str) -> bool {
        self.tools.contains_key(name)
    }

    pub fn names(&self) -> Vec<String> {
        self.tools.keys().cloned().collect()
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use serde_json::json;

    struct EchoTool;

    #[async_trait]
    impl Tool for EchoTool {
        fn definition(&self) -> ToolDefinition {
            ToolDefinition {
                name: "echo".to_string(),
                description: "Echoes back the input".to_string(),
                parameters: json!({"type": "object", "properties": {"text": {"type": "string"}}}),
            }
        }

        async fn execute(&self, input: Value) -> Result<ToolResult> {
            let text = input["text"].as_str().unwrap_or("(empty)");
            Ok(ToolResult::success(text))
        }
    }

    #[test]
    fn test_register_and_has() {
        let mut reg = ToolRegistry::new();
        assert!(!reg.has("echo"));
        reg.register(Arc::new(EchoTool));
        assert!(reg.has("echo"));
    }

    #[test]
    fn test_definitions() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(EchoTool));
        let defs = reg.definitions();
        assert_eq!(defs.len(), 1);
        assert_eq!(defs[0].name, "echo");
    }

    #[tokio::test]
    async fn test_dispatch_success() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(EchoTool));
        let result = reg.dispatch("echo", json!({"text": "hello"})).await.unwrap();
        assert_eq!(result.output, "hello");
        assert!(!result.is_error);
    }

    #[tokio::test]
    async fn test_dispatch_unknown_tool() {
        let reg = ToolRegistry::new();
        let result = reg.dispatch("nonexistent", json!({})).await;
        assert!(result.is_err());
    }
}
