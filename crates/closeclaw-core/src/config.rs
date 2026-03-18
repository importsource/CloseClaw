use serde::{Deserialize, Serialize};
use std::path::PathBuf;

#[derive(Debug, Clone, Deserialize)]
pub struct Config {
    pub gateway: GatewayConfig,
    pub agents: Vec<AgentConfig>,
    #[serde(default)]
    pub channels: Vec<ChannelConfig>,
    pub llm: LlmConfig,
    #[serde(default = "default_workspace")]
    pub workspace: PathBuf,
    #[serde(default)]
    pub schedules: Vec<ScheduleConfig>,
}

fn default_workspace() -> PathBuf {
    PathBuf::from(".")
}

#[derive(Debug, Clone, Deserialize)]
pub struct GatewayConfig {
    #[serde(default = "default_bind")]
    pub bind: String,
    #[serde(default = "default_port")]
    pub port: u16,
}

fn default_bind() -> String {
    "127.0.0.1".to_string()
}

fn default_port() -> u16 {
    3000
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            bind: default_bind(),
            port: default_port(),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgentConfig {
    pub id: String,
    #[serde(default)]
    pub name: Option<String>,
    #[serde(default)]
    pub soul_md: Option<PathBuf>,
    #[serde(default)]
    pub tools: Vec<String>,
    #[serde(default)]
    pub skills_dir: Option<PathBuf>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ChannelConfig {
    #[serde(rename = "type")]
    pub channel_type: ChannelType,
    #[serde(default)]
    pub enabled: Option<bool>,
    #[serde(default)]
    pub token_env: Option<String>,
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ChannelType {
    Cli,
    Webchat,
    Telegram,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScheduleConfig {
    pub id: String,
    /// 7-field cron expression: sec min hour dom month dow year
    pub cron: String,
    #[serde(default = "default_agent_id")]
    pub agent_id: String,
    pub message: String,
    #[serde(default = "default_enabled")]
    pub enabled: bool,
}

fn default_agent_id() -> String {
    "default".to_string()
}

fn default_enabled() -> bool {
    true
}

#[derive(Debug, Clone, Deserialize)]
pub struct LlmConfig {
    #[serde(default = "default_provider")]
    pub provider: LlmProvider,
    #[serde(default = "default_model")]
    pub model: String,
    #[serde(default = "default_auth_mode")]
    pub auth_mode: AuthMode,
    #[serde(default)]
    pub api_key_env: Option<String>,
    #[serde(default)]
    pub token_env: Option<String>,
    #[serde(default)]
    pub base_url: Option<String>,
    #[serde(default = "default_max_iterations")]
    pub max_iterations: usize,
}

/// Authentication mode for the LLM provider.
#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum AuthMode {
    /// Traditional API key auth (x-api-key header for Anthropic, Bearer for OpenAI)
    ApiKey,
    /// OAuth token auth (Authorization: Bearer header) — compatible with Claude Code's login flow
    OauthToken,
}

fn default_auth_mode() -> AuthMode {
    AuthMode::ApiKey
}

fn default_provider() -> LlmProvider {
    LlmProvider::Anthropic
}

fn default_model() -> String {
    "claude-sonnet-4-20250514".to_string()
}

fn default_max_iterations() -> usize {
    25
}

#[derive(Debug, Clone, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum LlmProvider {
    Anthropic,
    Openai,
}

impl Config {
    pub fn from_toml(content: &str) -> Result<Self, toml::de::Error> {
        toml::from_str(content)
    }

    pub fn from_file(path: &std::path::Path) -> Result<Self, Box<dyn std::error::Error + Send + Sync>> {
        let content = std::fs::read_to_string(path)?;
        Ok(Self::from_toml(&content)?)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_full_config() {
        let toml = r#"
[gateway]
bind = "0.0.0.0"
port = 8080

[[agents]]
id = "default"
name = "Test Agent"
tools = ["exec", "read_file"]

[[channels]]
type = "cli"
enabled = true

[[channels]]
type = "webchat"

[llm]
provider = "anthropic"
model = "claude-sonnet-4-20250514"
api_key_env = "MY_KEY"
max_iterations = 10
"#;
        let config = Config::from_toml(toml).unwrap();
        assert_eq!(config.gateway.bind, "0.0.0.0");
        assert_eq!(config.gateway.port, 8080);
        assert_eq!(config.agents.len(), 1);
        assert_eq!(config.agents[0].id, "default");
        assert_eq!(config.agents[0].tools, vec!["exec", "read_file"]);
        assert_eq!(config.channels.len(), 2);
        assert_eq!(config.channels[0].channel_type, ChannelType::Cli);
        assert_eq!(config.channels[1].channel_type, ChannelType::Webchat);
        assert_eq!(config.llm.provider, LlmProvider::Anthropic);
        assert_eq!(config.llm.max_iterations, 10);
    }

    #[test]
    fn test_parse_minimal_config() {
        let toml = r#"
[gateway]

[[agents]]
id = "a1"

[llm]
"#;
        let config = Config::from_toml(toml).unwrap();
        assert_eq!(config.gateway.bind, "127.0.0.1");
        assert_eq!(config.gateway.port, 3000);
        assert_eq!(config.llm.provider, LlmProvider::Anthropic);
        assert_eq!(config.llm.max_iterations, 25);
    }

    #[test]
    fn test_openai_provider() {
        let toml = r#"
[gateway]

[[agents]]
id = "x"

[llm]
provider = "openai"
model = "gpt-4o"
"#;
        let config = Config::from_toml(toml).unwrap();
        assert_eq!(config.llm.provider, LlmProvider::Openai);
        assert_eq!(config.llm.model, "gpt-4o");
    }

    #[test]
    fn test_oauth_token_mode() {
        let toml = r#"
[gateway]

[[agents]]
id = "default"

[llm]
provider = "anthropic"
auth_mode = "oauth_token"
token_env = "MY_CLAUDE_TOKEN"
"#;
        let config = Config::from_toml(toml).unwrap();
        assert_eq!(config.llm.auth_mode, AuthMode::OauthToken);
        assert_eq!(config.llm.token_env.as_deref(), Some("MY_CLAUDE_TOKEN"));
    }

    #[test]
    fn test_telegram_channel_config() {
        let toml = r#"
[gateway]

[[agents]]
id = "default"

[[channels]]
type = "telegram"
enabled = true
token_env = "MY_BOT_TOKEN"

[llm]
"#;
        let config = Config::from_toml(toml).unwrap();
        assert_eq!(config.channels.len(), 1);
        assert_eq!(config.channels[0].channel_type, ChannelType::Telegram);
        assert_eq!(config.channels[0].token_env.as_deref(), Some("MY_BOT_TOKEN"));
    }

    #[test]
    fn test_default_auth_mode_is_api_key() {
        let toml = r#"
[gateway]

[[agents]]
id = "a1"

[llm]
"#;
        let config = Config::from_toml(toml).unwrap();
        assert_eq!(config.llm.auth_mode, AuthMode::ApiKey);
    }

    #[test]
    fn test_parse_schedules() {
        let toml = r#"
[gateway]

[[agents]]
id = "default"

[[schedules]]
id = "morning-news"
cron = "0 0 9 * * 1-5 *"
agent_id = "default"
message = "Give me a news briefing"
enabled = true

[[schedules]]
id = "reminder"
cron = "0 30 14 * * * *"
message = "Remind me to stretch"

[llm]
"#;
        let config = Config::from_toml(toml).unwrap();
        assert_eq!(config.schedules.len(), 2);
        assert_eq!(config.schedules[0].id, "morning-news");
        assert_eq!(config.schedules[0].cron, "0 0 9 * * 1-5 *");
        assert_eq!(config.schedules[0].agent_id, "default");
        assert!(config.schedules[0].enabled);
        // Second schedule uses defaults for agent_id and enabled
        assert_eq!(config.schedules[1].id, "reminder");
        assert_eq!(config.schedules[1].agent_id, "default");
        assert!(config.schedules[1].enabled);
    }

    #[test]
    fn test_no_schedules_defaults_to_empty() {
        let toml = r#"
[gateway]

[[agents]]
id = "a1"

[llm]
"#;
        let config = Config::from_toml(toml).unwrap();
        assert!(config.schedules.is_empty());
    }
}
