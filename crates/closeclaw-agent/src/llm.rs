use async_trait::async_trait;
use closeclaw_core::error::{CloseClawError, Result};
use closeclaw_core::tool::ToolDefinition;
use closeclaw_core::types::ChatMessage;
use serde::Deserialize;
use serde_json::Value;
use sha2::{Digest, Sha256};

#[derive(Debug, Clone)]
pub struct ToolCall {
    pub id: String,
    pub name: String,
    pub input: Value,
}

#[derive(Debug, Clone)]
pub enum LlmResponse {
    Text(String),
    ToolUse(Vec<ToolCall>),
}

#[async_trait]
pub trait LlmProvider: Send + Sync {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse>;
}

// ── Anthropic Auth ───────────────────────────────────────────────────

/// How to authenticate with the Anthropic API.
#[derive(Debug, Clone)]
pub enum AnthropicAuth {
    /// Classic API key — sent as `x-api-key` header.
    ApiKey(String),
    /// OAuth Bearer token (Claude Code compatible) — sent as `Authorization: Bearer <token>`.
    OAuthToken(String),
}

// ── Anthropic ────────────────────────────────────────────────────────

pub struct AnthropicProvider {
    client: reqwest::Client,
    auth: AnthropicAuth,
    model: String,
    base_url: String,
}

// ── OAuth compatibility constants ────────────────────────────────────
const CC_VERSION: &str = "2.1.76";
const BILLING_SALT: &str = "59cf53e54c78";
const SYSTEM_PREFIX: &str = "You are Claude Code, Anthropic's official CLI for Claude.\n\n";

/// Compute the `x-anthropic-billing-header` value for OAuth requests.
/// Samples chars at indices 4, 7, 20 from concatenated user message text,
/// hashes with SHA-256 using a salt, and returns the first 3 hex chars.
fn compute_billing_hash(messages: &[ChatMessage]) -> String {
    let mut user_text = String::new();
    for msg in messages {
        if let ChatMessage::User(text) = msg {
            user_text.push_str(text);
        }
    }

    let sample: String = [4, 7, 20]
        .iter()
        .map(|&i| {
            user_text
                .chars()
                .nth(i)
                .unwrap_or('0')
        })
        .collect();

    let input = format!("{BILLING_SALT}{sample}{CC_VERSION}");
    let digest = Sha256::digest(input.as_bytes());
    let full_hex = hex::encode(digest);
    let hash = &full_hex[..3];

    format!(
        "cc_version={CC_VERSION}.{hash}; cc_entrypoint=cli; cch=1cfa3;"
    )
}

impl AnthropicProvider {
    /// Create with a classic API key.
    pub fn new(api_key: String, model: String, base_url: Option<String>) -> Self {
        Self::with_auth(AnthropicAuth::ApiKey(api_key), model, base_url)
    }

    /// Create with an explicit auth strategy (API key or OAuth token).
    pub fn with_auth(auth: AnthropicAuth, model: String, base_url: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            auth,
            model,
            base_url: base_url.unwrap_or_else(|| "https://api.anthropic.com".to_string()),
        }
    }

    fn convert_messages(messages: &[ChatMessage]) -> (Option<String>, Vec<Value>) {
        let mut system_prompt = None;
        let mut api_msgs: Vec<Value> = Vec::new();

        for msg in messages {
            match msg {
                ChatMessage::System(text) => {
                    system_prompt = Some(text.clone());
                }
                ChatMessage::User(text) => {
                    api_msgs.push(serde_json::json!({
                        "role": "user",
                        "content": text,
                    }));
                }
                ChatMessage::Assistant(text) => {
                    api_msgs.push(serde_json::json!({
                        "role": "assistant",
                        "content": text,
                    }));
                }
                ChatMessage::ToolUse { id, name, input } => {
                    api_msgs.push(serde_json::json!({
                        "role": "assistant",
                        "content": [{
                            "type": "tool_use",
                            "id": id,
                            "name": name,
                            "input": input,
                        }],
                    }));
                }
                ChatMessage::ToolResult {
                    id,
                    output,
                    is_error,
                } => {
                    api_msgs.push(serde_json::json!({
                        "role": "user",
                        "content": [{
                            "type": "tool_result",
                            "tool_use_id": id,
                            "content": output,
                            "is_error": is_error,
                        }],
                    }));
                }
            }
        }

        (system_prompt, api_msgs)
    }

    fn convert_tools(tools: &[ToolDefinition]) -> Vec<Value> {
        tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "name": t.name,
                    "description": t.description,
                    "input_schema": t.parameters,
                })
            })
            .collect()
    }
}

#[derive(Deserialize)]
struct AnthropicResponse {
    content: Vec<AnthropicContentBlock>,
    #[serde(default)]
    #[allow(dead_code)]
    stop_reason: Option<String>,
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum AnthropicContentBlock {
    #[serde(rename = "text")]
    Text { text: String },
    #[serde(rename = "tool_use")]
    ToolUse {
        id: String,
        name: String,
        input: Value,
    },
}

#[async_trait]
impl LlmProvider for AnthropicProvider {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse> {
        let is_oauth = matches!(self.auth, AnthropicAuth::OAuthToken(_));

        let (system, mut api_msgs) = Self::convert_messages(messages);
        let api_tools = Self::convert_tools(tools);

        // OAuth: prefix tool names with "mcp_" in messages that reference tools
        if is_oauth {
            for msg in &mut api_msgs {
                if let Some(content) = msg.get_mut("content") {
                    if let Some(arr) = content.as_array_mut() {
                        for block in arr.iter_mut() {
                            // tool_use blocks in assistant messages
                            if block.get("type").and_then(|v| v.as_str()) == Some("tool_use") {
                                if let Some(name) = block.get("name").and_then(|v| v.as_str()) {
                                    if !name.starts_with("mcp_") {
                                        block["name"] = Value::String(format!("mcp_{name}"));
                                    }
                                }
                            }
                            // tool_result blocks reference tool_use_id, no name change needed
                        }
                    }
                }
            }
        }

        let mut body = serde_json::json!({
            "model": self.model,
            "max_tokens": 4096,
            "messages": api_msgs,
        });

        if let Some(sys) = system {
            if is_oauth {
                // OAuth: approved prefix must be a separate first block;
                // additional content goes in a second block.
                body["system"] = serde_json::json!([
                    {"type": "text", "text": SYSTEM_PREFIX.trim()},
                    {"type": "text", "text": sys},
                ]);
            } else {
                body["system"] = Value::String(sys);
            }
        } else if is_oauth {
            // Even without a user system prompt, we need the prefix for OAuth
            body["system"] = serde_json::json!([{
                "type": "text",
                "text": SYSTEM_PREFIX.trim(),
            }]);
        }

        if !api_tools.is_empty() {
            if is_oauth {
                // OAuth: prefix tool names with "mcp_"
                let prefixed: Vec<Value> = api_tools
                    .into_iter()
                    .map(|mut t| {
                        if let Some(name) = t.get("name").and_then(|v| v.as_str()) {
                            if !name.starts_with("mcp_") {
                                t["name"] = Value::String(format!("mcp_{name}"));
                            }
                        }
                        t
                    })
                    .collect();
                body["tools"] = Value::Array(prefixed);
            } else {
                body["tools"] = Value::Array(api_tools);
            }
        }

        // Build URL
        let url = if is_oauth {
            format!("{}/v1/messages?beta=true", self.base_url)
        } else {
            format!("{}/v1/messages", self.base_url)
        };

        let mut req = self
            .client
            .post(&url)
            .header("anthropic-version", "2023-06-01")
            .header("content-type", "application/json");

        req = match &self.auth {
            AnthropicAuth::ApiKey(key) => req.header("x-api-key", key),
            AnthropicAuth::OAuthToken(token) => {
                let billing = compute_billing_hash(messages);
                req.header("Authorization", format!("Bearer {token}"))
                    .header(
                        "anthropic-beta",
                        "oauth-2025-04-20,interleaved-thinking-2025-05-14",
                    )
                    .header("user-agent", format!("claude-code/{CC_VERSION}"))
                    .header("x-anthropic-billing-header", billing)
            }
        };

        let resp = req
            .json(&body)
            .send()
            .await
            .map_err(|e| CloseClawError::Llm(format!("HTTP error: {e}")))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| CloseClawError::Llm(format!("Failed to read response: {e}")))?;

        if !status.is_success() {
            return Err(CloseClawError::Llm(format!(
                "Anthropic API error {status}: {text}"
            )));
        }

        let parsed: AnthropicResponse = serde_json::from_str(&text)
            .map_err(|e| CloseClawError::Llm(format!("Failed to parse response: {e}")))?;

        let mut tool_calls = Vec::new();
        let mut text_parts = Vec::new();

        for block in parsed.content {
            match block {
                AnthropicContentBlock::Text { text } => text_parts.push(text),
                AnthropicContentBlock::ToolUse { id, name, input } => {
                    // OAuth: strip "mcp_" prefix from tool names in response
                    let name = if is_oauth {
                        name.strip_prefix("mcp_")
                            .unwrap_or(&name)
                            .to_string()
                    } else {
                        name
                    };
                    tool_calls.push(ToolCall { id, name, input });
                }
            }
        }

        if !tool_calls.is_empty() {
            Ok(LlmResponse::ToolUse(tool_calls))
        } else {
            Ok(LlmResponse::Text(text_parts.join("\n")))
        }
    }
}

// ── OpenAI ────────────────────────────────────────────────────────────

pub struct OpenAiProvider {
    client: reqwest::Client,
    api_key: String,
    model: String,
    base_url: String,
}

impl OpenAiProvider {
    pub fn new(api_key: String, model: String, base_url: Option<String>) -> Self {
        Self {
            client: reqwest::Client::new(),
            api_key,
            model,
            base_url: base_url.unwrap_or_else(|| "https://api.openai.com".to_string()),
        }
    }

    fn convert_messages(messages: &[ChatMessage]) -> Vec<Value> {
        let mut api_msgs: Vec<Value> = Vec::new();

        for msg in messages {
            match msg {
                ChatMessage::System(text) => {
                    api_msgs.push(serde_json::json!({
                        "role": "system",
                        "content": text,
                    }));
                }
                ChatMessage::User(text) => {
                    api_msgs.push(serde_json::json!({
                        "role": "user",
                        "content": text,
                    }));
                }
                ChatMessage::Assistant(text) => {
                    api_msgs.push(serde_json::json!({
                        "role": "assistant",
                        "content": text,
                    }));
                }
                ChatMessage::ToolUse { id, name, input } => {
                    api_msgs.push(serde_json::json!({
                        "role": "assistant",
                        "tool_calls": [{
                            "id": id,
                            "type": "function",
                            "function": {
                                "name": name,
                                "arguments": input.to_string(),
                            }
                        }],
                    }));
                }
                ChatMessage::ToolResult {
                    id,
                    output,
                    is_error: _,
                } => {
                    api_msgs.push(serde_json::json!({
                        "role": "tool",
                        "tool_call_id": id,
                        "content": output,
                    }));
                }
            }
        }

        api_msgs
    }

    fn convert_tools(tools: &[ToolDefinition]) -> Vec<Value> {
        tools
            .iter()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name,
                        "description": t.description,
                        "parameters": t.parameters,
                    }
                })
            })
            .collect()
    }
}

#[derive(Deserialize)]
struct OpenAiResponse {
    choices: Vec<OpenAiChoice>,
}

#[derive(Deserialize)]
struct OpenAiChoice {
    message: OpenAiMessage,
}

#[derive(Deserialize)]
struct OpenAiMessage {
    content: Option<String>,
    #[serde(default)]
    tool_calls: Vec<OpenAiToolCall>,
}

#[derive(Deserialize)]
struct OpenAiToolCall {
    id: String,
    function: OpenAiFunction,
}

#[derive(Deserialize)]
struct OpenAiFunction {
    name: String,
    arguments: String,
}

#[async_trait]
impl LlmProvider for OpenAiProvider {
    async fn chat(
        &self,
        messages: &[ChatMessage],
        tools: &[ToolDefinition],
    ) -> Result<LlmResponse> {
        let api_msgs = Self::convert_messages(messages);
        let api_tools = Self::convert_tools(tools);

        let mut body = serde_json::json!({
            "model": self.model,
            "messages": api_msgs,
        });

        if !api_tools.is_empty() {
            body["tools"] = Value::Array(api_tools);
        }

        let resp = self
            .client
            .post(format!("{}/v1/chat/completions", self.base_url))
            .header("Authorization", format!("Bearer {}", self.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .send()
            .await
            .map_err(|e| CloseClawError::Llm(format!("HTTP error: {e}")))?;

        let status = resp.status();
        let text = resp
            .text()
            .await
            .map_err(|e| CloseClawError::Llm(format!("Failed to read response: {e}")))?;

        if !status.is_success() {
            return Err(CloseClawError::Llm(format!(
                "OpenAI API error {status}: {text}"
            )));
        }

        let parsed: OpenAiResponse = serde_json::from_str(&text)
            .map_err(|e| CloseClawError::Llm(format!("Failed to parse response: {e}")))?;

        let choice = parsed
            .choices
            .into_iter()
            .next()
            .ok_or_else(|| CloseClawError::Llm("No choices in response".into()))?;

        if !choice.message.tool_calls.is_empty() {
            let calls = choice
                .message
                .tool_calls
                .into_iter()
                .map(|tc| {
                    let input: Value =
                        serde_json::from_str(&tc.function.arguments).unwrap_or(Value::Null);
                    ToolCall {
                        id: tc.id,
                        name: tc.function.name,
                        input,
                    }
                })
                .collect();
            Ok(LlmResponse::ToolUse(calls))
        } else {
            Ok(LlmResponse::Text(
                choice.message.content.unwrap_or_default(),
            ))
        }
    }
}
