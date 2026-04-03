mod setup;

use anyhow::{Context, Result};
use clap::{Parser, Subcommand};
use closeclaw_agent::llm::{AnthropicAuth, AnthropicProvider, OpenAiProvider};
use closeclaw_agent::runtime::AgentRuntime;
use closeclaw_agent::tool_dispatch::ToolRegistry;
use closeclaw_channels::cli::CliChannel;
use closeclaw_channels::telegram::TelegramChannel;
use closeclaw_core::config::{AuthMode, ChannelType, Config, LlmProvider};
use closeclaw_core::schedule::ScheduleNotifier;
use closeclaw_core::types::AgentId;
use closeclaw_gateway::hub::Hub;
use closeclaw_gateway::schedule_store::ScheduleStore;
use closeclaw_gateway::scheduler::{ScheduleHandleImpl, Scheduler};
use std::path::{Path, PathBuf};
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

    load_dotenv();

    let cli = Cli::parse();

    match cli.command {
        Some(Command::Run) => {
            if needs_setup(&cli.config) {
                run_setup(cli.config).await
            } else {
                let config = load_config(&cli.config)?;
                let workspace = cli
                    .workspace
                    .unwrap_or_else(|| config.workspace.clone())
                    .canonicalize()
                    .unwrap_or_else(|_| PathBuf::from("."));
                info!("Workspace: {}", workspace.display());
                run_gateway(config, workspace, cli.config.clone()).await
            }
        }
        Some(Command::Chat) | Some(Command::Telegram) => {
            let config = load_config(&cli.config)?;
            let workspace = cli
                .workspace
                .unwrap_or_else(|| config.workspace.clone())
                .canonicalize()
                .unwrap_or_else(|_| PathBuf::from("."));
            info!("Workspace: {}", workspace.display());

            match cli.command.unwrap() {
                Command::Chat => run_chat(config, workspace).await,
                Command::Telegram => run_telegram(config, workspace).await,
                _ => unreachable!(),
            }
        }
        None => {
            if needs_setup(&cli.config) {
                run_setup(cli.config).await
            } else {
                let config = load_config(&cli.config)?;
                let workspace = cli
                    .workspace
                    .unwrap_or_else(|| config.workspace.clone())
                    .canonicalize()
                    .unwrap_or_else(|_| PathBuf::from("."));
                info!("Workspace: {}", workspace.display());
                run_chat(config, workspace).await
            }
        }
    }
}

/// Load `~/.closeclaw/.env` into process env vars (only if not already set).
fn load_dotenv() {
    let path = dirs_home().join(".closeclaw/.env");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return,
    };
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((key, value)) = line.split_once('=') {
            let key = key.trim();
            let value = value.trim();
            if std::env::var(key).is_err() {
                std::env::set_var(key, value);
            }
        }
    }
}

/// Returns true if the setup wizard should be shown.
fn needs_setup(config_path: &Path) -> bool {
    if !config_path.exists() {
        return true;
    }
    // Config exists — check if OAuth mode is configured (no API key needed)
    if let Ok(content) = std::fs::read_to_string(config_path) {
        if content.contains("auth_mode = \"oauth_token\"") {
            return false;
        }
    }
    // Otherwise need setup if no API key is available
    std::env::var("ANTHROPIC_API_KEY").is_err() && std::env::var("OPENAI_API_KEY").is_err()
}

/// Run the setup wizard, then start the gateway.
async fn run_setup(config_path: PathBuf) -> Result<()> {
    setup::serve_setup("127.0.0.1", 3000, config_path.clone()).await?;

    // Re-load the freshly-written config
    let config = Config::from_file(&config_path)
        .map_err(|e| anyhow::anyhow!("Failed to load config after setup: {e}"))?;
    let workspace = config
        .workspace
        .clone()
        .canonicalize()
        .unwrap_or_else(|_| PathBuf::from("."));
    info!("Workspace: {}", workspace.display());

    run_gateway(config, workspace, config_path).await
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
                "list_files".to_string(),
                "create_file".to_string(),
                "delete_file".to_string(),
                "search_files".to_string(),
                "browser".to_string(),
                "add_schedule".to_string(),
                "remove_schedule".to_string(),
                "list_schedules".to_string(),
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
        schedules: vec![],
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
///
/// Claude Code stores credentials under the service name "Claude Code-credentials"
/// as a JSON blob. The OAuth token lives at `.claudeAiOauth.accessToken`.
fn read_claude_code_keychain_token() -> Option<String> {
    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("security")
            .args(["find-generic-password", "-s", "Claude Code-credentials", "-w"])
            .output()
            .ok()?;

        if !output.status.success() {
            return None;
        }

        let raw = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if raw.is_empty() {
            return None;
        }

        // Parse JSON and extract the OAuth access token
        let json: serde_json::Value = serde_json::from_str(&raw).ok()?;
        let token = json
            .get("claudeAiOauth")?
            .get("accessToken")?
            .as_str()?
            .to_string();

        if token.is_empty() {
            None
        } else {
            Some(token)
        }
    }

    #[cfg(not(target_os = "macos"))]
    {
        tracing::warn!("Keychain token extraction is only supported on macOS");
        None
    }
}

/// Collect skill directories in priority order:
/// 1. `~/.closeclaw/skills/` — global user skills
/// 2. `{workspace}/skills/` — project-specific skills
/// 3. `agent_config.skills_dir` — custom per-agent override (if set)
fn collect_skills_dirs(workspace: &Path, config: &Config) -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    // Global user skills
    dirs.push(dirs_home().join(".closeclaw/skills"));

    // Project-specific skills
    dirs.push(workspace.join("skills"));

    // Per-agent custom skills_dir (use first agent's config)
    if let Some(agent) = config.agents.first() {
        if let Some(ref custom) = agent.skills_dir {
            dirs.push(custom.clone());
        }
    }

    dirs
}

fn build_tool_registry(
    workspace: &PathBuf,
    schedule_handle: Option<Arc<dyn closeclaw_core::schedule::ScheduleHandle>>,
) -> ToolRegistry {
    let mut registry = ToolRegistry::new();
    for tool in closeclaw_tools::builtin_tools(workspace) {
        registry.register(tool);
    }
    if let Some(handle) = schedule_handle {
        for tool in closeclaw_tools::schedule_tools(handle) {
            registry.register(tool);
        }
    }
    registry
}

async fn run_chat(config: Config, workspace: PathBuf) -> Result<()> {
    let llm = build_llm_provider(&config)?;
    let tools = Arc::new(build_tool_registry(&workspace, None));
    let skills_dirs = collect_skills_dirs(&workspace, &config);

    let agent_runtime = Arc::new(AgentRuntime::new(
        llm,
        tools,
        workspace.clone(),
        skills_dirs,
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

async fn run_gateway(config: Config, workspace: PathBuf, config_path: PathBuf) -> Result<()> {
    let llm = build_llm_provider(&config)?;

    // 1. Create mpsc channel for schedule commands
    let (sched_tx, sched_rx) = tokio::sync::mpsc::channel(64);
    let schedule_handle: Arc<dyn closeclaw_core::schedule::ScheduleHandle> =
        Arc::new(ScheduleHandleImpl::new(sched_tx));

    // 2. Build tool registry with schedule tools
    let tools = Arc::new(build_tool_registry(&workspace, Some(schedule_handle)));
    let skills_dirs = collect_skills_dirs(&workspace, &config);

    let agent_runtime = Arc::new(AgentRuntime::new(
        llm,
        tools,
        workspace.clone(),
        skills_dirs,
        config.llm.max_iterations,
    ));

    let skills = agent_runtime.skills.clone();

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
    let wc_skills = skills;
    let wc_config = config.clone();
    let wc_config_path = config_path;
    let wc_workspace = workspace.clone();
    handles.push(tokio::spawn(async move {
        if let Err(e) = closeclaw_channels::webchat::serve(
            hub_wc, wc_skills, wc_config, wc_config_path, wc_workspace, &bind, port,
        ).await {
            error!("WebChat error: {e}");
        }
    }));

    // Start telegram if configured and enabled
    for ch in &config.channels {
        if ch.channel_type == ChannelType::Telegram && ch.enabled.unwrap_or(true) {
            let token = resolve_telegram_token(ch.token_env.as_deref())?;
            let hub_tg = hub.clone();
            let ws = workspace.clone();
            handles.push(tokio::spawn(async move {
                TelegramChannel::new(token, ws).run(hub_tg).await;
            }));
        }
    }

    // Build notifier for schedule response delivery
    let notifier: Option<Arc<dyn ScheduleNotifier>> = {
        // If Telegram is enabled, create a notifier that delivers to Telegram chats
        let tg_token = config
            .channels
            .iter()
            .find(|c| c.channel_type == ChannelType::Telegram && c.enabled.unwrap_or(true))
            .and_then(|c| resolve_telegram_token(c.token_env.as_deref()).ok());
        tg_token.map(|token| {
            Arc::new(TelegramNotifier {
                bot: teloxide::Bot::new(token),
            }) as Arc<dyn ScheduleNotifier>
        })
    };

    // Always start scheduler (dynamic schedules may arrive via tools)
    let store = ScheduleStore::new(dirs_home().join(".closeclaw/schedules.json"));
    let scheduler = Scheduler::new(&config.schedules, store, sched_rx, notifier);
    scheduler.restore_sessions(&hub).await;
    let hub_sched = hub.clone();
    let (_shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
    handles.push(tokio::spawn(async move {
        scheduler.run(hub_sched, shutdown_rx).await;
    }));

    // If we just restarted, notify connected clients after a short delay
    let restart_marker = closeclaw_tools::self_manage::restart_marker_path();
    if restart_marker.exists() {
        let _ = std::fs::remove_file(&restart_marker);
        let bus = hub.event_sender();
        tokio::spawn(async move {
            // Wait for WebChat clients to reconnect before broadcasting
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            let _ = bus.send(closeclaw_core::types::Event::SystemNotice {
                message: "Server has restarted successfully.".to_string(),
            });
        });
    }

    // Wait for any channel to finish (typically they run forever)
    for handle in handles {
        handle.await.ok();
    }

    Ok(())
}

async fn run_telegram(config: Config, workspace: PathBuf) -> Result<()> {
    let llm = build_llm_provider(&config)?;
    let tools = Arc::new(build_tool_registry(&workspace, None));
    let skills_dirs = collect_skills_dirs(&workspace, &config);

    let agent_runtime = Arc::new(AgentRuntime::new(
        llm,
        tools,
        workspace.clone(),
        skills_dirs,
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
    TelegramChannel::new(token, workspace).run(hub).await;

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

/// Delivers scheduled task responses to Telegram chats.
/// Parses the peer_id (format "tg:{chat_id}") to extract the Telegram chat ID.
struct TelegramNotifier {
    bot: teloxide::Bot,
}

#[async_trait::async_trait]
impl ScheduleNotifier for TelegramNotifier {
    async fn notify(&self, schedule_id: &str, peer_id: &str, response: &str) {
        // peer_id format from Telegram channel: "tg:{chat_id}"
        let chat_id = match peer_id.strip_prefix("tg:").and_then(|s| s.parse::<i64>().ok()) {
            Some(id) => teloxide::types::ChatId(id),
            None => {
                tracing::warn!(
                    schedule_id = %schedule_id,
                    peer_id = %peer_id,
                    "Cannot deliver schedule notification: peer_id is not a Telegram chat"
                );
                return;
            }
        };

        closeclaw_channels::telegram::send_html(&self.bot, chat_id, response).await;

        info!(
            schedule_id = %schedule_id,
            chat_id = %chat_id,
            "Delivered scheduled response to Telegram"
        );
    }
}
