use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use closeclaw_agent::llm::{AnthropicAuth, AnthropicProvider, OpenAiProvider};
use closeclaw_agent::runtime::AgentRuntime;
use closeclaw_agent::tool_dispatch::ToolRegistry;
use closeclaw_channels::cli::CliChannel;
use closeclaw_channels::telegram::TelegramChannel;
use closeclaw_core::config::{AuthMode, ChannelType, Config, LlmProvider};
use closeclaw_core::types::AgentId;
use closeclaw_gateway::hub::Hub;
use std::path::PathBuf;
use std::sync::Arc;
use tracing::{error, info};

#[derive(Parser)]
#[command(name = "closeclaw", about = "CloseClaw — An agent framework in Rust")]
struct Cli {
    /// Path to configuration file
    #[arg(short, long, default_value = "config.toml")]
    config: PathBuf,

    /// Workspace directory
    #[arg(short, long)]
    workspace: Option<PathBuf>,

    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Subcommand)]
enum Command {
    /// Start the gateway with all enabled channels
    Run,
    /// Direct CLI chat mode (single agent, no gateway overhead)
    Chat,
    /// Standalone Telegram bot mode
    Telegram,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

    let config = load_config(&cli.config)?;
    let workspace = cli
        .workspace
        .unwrap_or_else(|| config.workspace.clone())
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("."));

    info!("Workspace: {}", workspace.display());

    match cli.command.unwrap_or(Command::Chat) {
        Command::Chat => run_chat(config, workspace).await,
        Command::Run => run_gateway(config, workspace).await,
        Command::Telegram => run_telegram(config, workspace).await,
    }
}

fn load_config(path: &PathBuf) -> Result<Config> {
    if path.exists() {
        Config::from_file(path)
            .map_err(|e| anyhow::anyhow!("Failed to load config from {}: {e}", path.display()))
    } else {
        info!("No config file found, using defaults");
        Ok(default_config())
    }
}

fn default_config() -> Config {
    Config {
        gateway: closeclaw_core::config::GatewayConfig::default(),
        agents: vec![closeclaw_core::config::AgentConfig {
            id: "default".to_string(),
            name: Some("CloseClaw Agent".to_string()),
            soul_md: None,
            tools: vec![
                "exec".to_string(),
                "read_file".to_string(),
                "write_file".to_string(),
                "web_fetch".to_string(),
                "web_search".to_string(),
            ],
            skills_dir: None,
        }],
        channels: vec![],
        llm: closeclaw_core::config::LlmConfig {
            provider: LlmProvider::Anthropic,
            model: "claude-sonnet-4-20250514".to_string(),
            auth_mode: AuthMode::ApiKey,
            api_key_env: Some("ANTHROPIC_API_KEY".to_string()),
            token_env: None,
            base_url: None,
            max_iterations: 25,
        },
        workspace: PathBuf::from("."),
    }
}

fn build_llm_provider(config: &Config) -> Result<Arc<dyn closeclaw_agent::llm::LlmProvider>> {
    let provider: Arc<dyn closeclaw_agent::llm::LlmProvider> = match config.llm.provider {
        LlmProvider::Anthropic => {
            let auth = resolve_anthropic_auth(config)?;
            Arc::new(AnthropicProvider::with_auth(
                auth,
                config.llm.model.clone(),
                config.llm.base_url.clone(),
            ))
        }
        LlmProvider::Openai => {
            let env_var = config.llm.api_key_env.as_deref().unwrap_or("OPENAI_API_KEY");
            let api_key = std::env::var(env_var)
                .with_context(|| format!("Missing environment variable: {env_var}"))?;
            Arc::new(OpenAiProvider::new(
                api_key,
                config.llm.model.clone(),
                config.llm.base_url.clone(),
            ))
        }
    };

    Ok(provider)
}

/// Resolve Anthropic authentication based on the configured auth_mode.
///
/// Auth modes:
///   - `api_key` (default): reads from `api_key_env` or ANTHROPIC_API_KEY
///   - `oauth_token`: reads from `token_env` or CLAUDE_CODE_TOKEN, falling back
///      to extracting from the macOS Keychain (where Claude Code stores its OAuth tokens)
fn resolve_anthropic_auth(config: &Config) -> Result<AnthropicAuth> {
    match config.llm.auth_mode {
        AuthMode::ApiKey => {
            let env_var = config.llm.api_key_env.as_deref().unwrap_or("ANTHROPIC_API_KEY");
            let key = std::env::var(env_var)
                .with_context(|| format!("Missing environment variable: {env_var}"))?;
            info!("Using Anthropic API key auth");
            Ok(AnthropicAuth::ApiKey(key))
        }
        AuthMode::OauthToken => {
            // 1. Try explicit env var
            let env_var = config.llm.token_env.as_deref().unwrap_or("CLAUDE_CODE_TOKEN");
            if let Ok(token) = std::env::var(env_var) {
                info!("Using OAuth token from {env_var}");
                return Ok(AnthropicAuth::OAuthToken(token));
            }

            // 2. Try reading from macOS Keychain (where Claude Code stores tokens)
            if let Some(token) = read_claude_code_keychain_token() {
                info!("Using OAuth token from Claude Code keychain");
                return Ok(AnthropicAuth::OAuthToken(token));
            }

            anyhow::bail!(
                "OAuth token mode is configured but no token found.\n\
                 Set {env_var} or log in with Claude Code (`claude login`) to store a token in the keychain."
            )
        }
    }
}

/// Attempt to read Claude Code's OAuth token from the macOS Keychain.
/// Returns None on non-macOS platforms or if the keychain entry doesn't exist.
fn read_claude_code_keychain_token() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        // Claude Code stores its OAuth token under the service name "claude.ai"
        // in the user's login keychain. We try common service names.
        for service in &["claude.ai", "claude-code", "api.anthropic.com"] {
            let output = std::process::Command::new("security")
                .args(["find-generic-password", "-s", service, "-w"])
                .output()
                .ok()?;

            if output.status.success() {
                let token = String::from_utf8_lossy(&output.stdout).trim().to_string();
                if !token.is_empty() {
                    return Some(token);
                }
            }
        }
        None
    }

    #[cfg(not(target_os = "macos"))]
    {
        warn!("Keychain token extraction is only supported on macOS");
        None
    }
}

fn build_tool_registry(workspace: &PathBuf) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    for tool in closeclaw_tools::builtin_tools(workspace) {
        registry.register(tool);
    }
    registry
}

async fn run_chat(config: Config, workspace: PathBuf) -> Result<()> {
    let llm = build_llm_provider(&config)?;
    let tools = Arc::new(build_tool_registry(&workspace));

    let agent_runtime = Arc::new(AgentRuntime::new(
        llm,
        tools,
        workspace.clone(),
        config.llm.max_iterations,
    ));

    let agent_id = AgentId("default".to_string());

    let session_dir = dirs_home().join(".closeclaw/sessions");
    let hub = Arc::new(Hub::new(agent_id.clone(), session_dir));
    hub.init().await?;
    hub.register_agent(agent_id, agent_runtime);

    let cli_channel = CliChannel::new();
    cli_channel.run(hub).await;

    Ok(())
}

async fn run_gateway(config: Config, workspace: PathBuf) -> Result<()> {
    let llm = build_llm_provider(&config)?;
    let tools = Arc::new(build_tool_registry(&workspace));

    let agent_runtime = Arc::new(AgentRuntime::new(
        llm,
        tools,
        workspace.clone(),
        config.llm.max_iterations,
    ));

    let agent_id = AgentId("default".to_string());

    let session_dir = dirs_home().join(".closeclaw/sessions");
    let hub = Arc::new(Hub::new(agent_id.clone(), session_dir));
    hub.init().await?;
    hub.register_agent(agent_id, agent_runtime);

    info!("Starting CloseClaw gateway with enabled channels");

    let mut handles = Vec::new();

    // Always start webchat
    let bind = config.gateway.bind.clone();
    let port = config.gateway.port;
    let hub_wc = hub.clone();
    handles.push(tokio::spawn(async move {
        if let Err(e) = closeclaw_channels::webchat::serve(hub_wc, &bind, port).await {
            error!("WebChat error: {e}");
        }
    }));

    // Start telegram if configured and enabled
    for ch in &config.channels {
        if ch.channel_type == ChannelType::Telegram && ch.enabled.unwrap_or(true) {
            let token = resolve_telegram_token(ch.token_env.as_deref())?;
            let hub_tg = hub.clone();
            handles.push(tokio::spawn(async move {
                TelegramChannel::new(token).run(hub_tg).await;
            }));
        }
    }

    // Wait for any channel to finish (typically they run forever)
    for handle in handles {
        handle.await.ok();
    }

    Ok(())
}

async fn run_telegram(config: Config, workspace: PathBuf) -> Result<()> {
    let llm = build_llm_provider(&config)?;
    let tools = Arc::new(build_tool_registry(&workspace));

    let agent_runtime = Arc::new(AgentRuntime::new(
        llm,
        tools,
        workspace.clone(),
        config.llm.max_iterations,
    ));

    let agent_id = AgentId("default".to_string());

    let session_dir = dirs_home().join(".closeclaw/sessions");
    let hub = Arc::new(Hub::new(agent_id.clone(), session_dir));
    hub.init().await?;
    hub.register_agent(agent_id, agent_runtime);

    // Find telegram channel config, or use defaults
    let token_env = config
        .channels
        .iter()
        .find(|c| c.channel_type == ChannelType::Telegram)
        .and_then(|c| c.token_env.as_deref())
        .or(Some("TELOXIDE_TOKEN"));

    let token = resolve_telegram_token(token_env)?;

    info!("Starting standalone Telegram bot");
    TelegramChannel::new(token).run(hub).await;

    Ok(())
}

fn resolve_telegram_token(env_name: Option<&str>) -> Result<String> {
    let env_var = env_name.unwrap_or("TELOXIDE_TOKEN");
    std::env::var(env_var)
        .with_context(|| format!("Missing Telegram bot token. Set {env_var} (get one from @BotFather on Telegram)"))
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}
