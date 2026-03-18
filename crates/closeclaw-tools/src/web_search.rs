use async_trait::async_trait;
use closeclaw_core::error::Result;
use closeclaw_core::tool::{Tool, ToolDefinition, ToolResult};
use serde_json::{json, Value};

/// Placeholder web search tool. In production, this would integrate with
/// a search API (e.g., Brave Search, Tavily, SerpAPI).
pub struct WebSearchTool;

impl WebSearchTool {
    pub fn new() -> Self {
        Self
    }
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "web_search".to_string(),
            description: "Search the web for information. Returns search results with titles, URLs, and snippets.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query"
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn execute(&self, input: Value) -> Result<ToolResult> {
        let query = input["query"]
            .as_str()
            .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool("missing 'query' field".into()))?;

        // Placeholder — returns a message indicating no search API is configured
        Ok(ToolResult::error(format!(
            "Web search is not configured. Query was: \"{query}\". \
             To enable web search, configure a search API provider in config.toml."
        )))
    }
}
