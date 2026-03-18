use async_trait::async_trait;
use closeclaw_core::error::Result;
use closeclaw_core::tool::{Tool, ToolDefinition, ToolResult};
use serde_json::{json, Value};

pub struct WebFetchTool {
    client: reqwest::Client,
}

impl WebFetchTool {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(30))
                .build()
                .expect("failed to build HTTP client"),
        }
    }
}

impl Default for WebFetchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WebFetchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "web_fetch".to_string(),
            description: "Fetch the content of a URL and return it as text.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "url": {
                        "type": "string",
                        "description": "The URL to fetch"
                    }
                },
                "required": ["url"]
            }),
        }
    }

    async fn execute(&self, input: Value) -> Result<ToolResult> {
        let url = input["url"]
            .as_str()
            .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool("missing 'url' field".into()))?;

        let response = match self.client.get(url).send().await {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::error(format!("HTTP request failed: {e}"))),
        };

        let status = response.status();
        if !status.is_success() {
            return Ok(ToolResult::error(format!("HTTP {status}")));
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        let body = match response.text().await {
            Ok(b) => b,
            Err(e) => return Ok(ToolResult::error(format!("Failed to read body: {e}"))),
        };

        let text = if content_type.contains("text/html") {
            html2text::from_read(body.as_bytes(), 80)
                .unwrap_or_else(|_| body.clone())
        } else {
            body
        };

        // Truncate very long responses
        let max_len = 50_000;
        let output = if text.len() > max_len {
            format!("{}...\n[truncated, {} total chars]", &text[..max_len], text.len())
        } else {
            text
        };

        Ok(ToolResult::success(output))
    }
}
