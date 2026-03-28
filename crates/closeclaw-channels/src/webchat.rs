use axum::{
    extract::{
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
        State,
    },
    http::StatusCode,
    response::{Html, IntoResponse, Json},
    routing::{get, post},
    Router,
};
use closeclaw_core::config::Config;
use closeclaw_core::skill::Skill;
use closeclaw_core::types::{
    ChannelId, Event, Message, MessageContent, Sender, SessionId,
};
use closeclaw_gateway::hub::Hub;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, info};

#[derive(Clone)]
struct AppState {
    hub: Arc<Hub>,
    skills: Arc<Vec<Skill>>,
    config: Arc<RwLock<Config>>,
    config_path: Arc<PathBuf>,
    workspace: Arc<PathBuf>,
}

#[derive(Serialize, Deserialize)]
struct ChatMsg {
    #[serde(rename = "type")]
    msg_type: String,
    content: String,
}

pub async fn serve(
    hub: Arc<Hub>,
    skills: Vec<Skill>,
    config: Config,
    config_path: PathBuf,
    workspace: PathBuf,
    bind: &str,
    port: u16,
) -> anyhow::Result<()> {
    let state = AppState {
        hub,
        skills: Arc::new(skills),
        config: Arc::new(RwLock::new(config)),
        config_path: Arc::new(config_path),
        workspace: Arc::new(workspace),
    };
    let app = Router::new()
        .route("/", get(index_handler))
        .route("/ws", get(ws_handler))
        .route("/api/skills", get(skills_handler))
        .route("/api/config", get(config_get_handler))
        .route("/api/config", post(config_post_handler))
        .route("/favicon.ico", get(favicon_handler))
        .route("/logo.png", get(logo_handler))
        .route("/files/{*path}", get(file_handler))
        .with_state(state);

    let addr = format!("{bind}:{port}");
    info!("WebChat listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index_handler(State(state): State<AppState>) -> Html<String> {
    let workspace_str = state.workspace.display().to_string();
    let html = CHAT_HTML.replace("__WORKSPACE__", &workspace_str);
    Html(html)
}

async fn file_handler(
    axum::extract::Path(path): axum::extract::Path<String>,
) -> impl axum::response::IntoResponse {
    // Resolve the path: expand ~ to home dir, or treat as absolute
    let file_path = if path.starts_with("~/") || path == "~" {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".to_string());
        PathBuf::from(home).join(path.strip_prefix("~/").unwrap_or(&path))
    } else {
        PathBuf::from(format!("/{path}"))
    };

    let canonical = match file_path.canonicalize() {
        Ok(p) => p,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };

    // Only serve image files for security
    let ext = canonical
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    let content_type = match ext.as_str() {
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        _ => return (StatusCode::FORBIDDEN, "Only image files are served").into_response(),
    };

    let bytes = match std::fs::read(&canonical) {
        Ok(b) => b,
        Err(_) => return (StatusCode::NOT_FOUND, "Not found").into_response(),
    };

    (
        [(axum::http::header::CONTENT_TYPE, content_type)],
        bytes,
    )
        .into_response()
}

async fn favicon_handler() -> impl axum::response::IntoResponse {
    static FAVICON: &[u8] = include_bytes!("../../../favicon.ico");
    ([(axum::http::header::CONTENT_TYPE, "image/x-icon")], FAVICON)
}

async fn logo_handler() -> impl axum::response::IntoResponse {
    static LOGO: &[u8] = include_bytes!("../../../closeclaw-icon.png");
    ([(axum::http::header::CONTENT_TYPE, "image/png")], LOGO)
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl axum::response::IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: AppState) {
    let own_sid = SessionId(uuid::Uuid::new_v4().to_string());
    let channel_id = ChannelId("webchat".to_string());
    let user_id = uuid::Uuid::new_v4().to_string();

    info!("WebSocket connected: session {own_sid}");

    // Subscribe once at connection start so we never miss cross-channel events
    let mut event_rx = state.hub.subscribe_events();

    // Track foreign sessions we are actively relaying
    let mut tracked_foreign: HashSet<SessionId> = HashSet::new();

    // Optional oneshot for the own-session response currently in flight
    let mut own_result_rx: Option<tokio::sync::oneshot::Receiver<closeclaw_core::error::Result<String>>> = None;

    loop {
        tokio::select! {
            // ── Branch 1: incoming WebSocket message from browser ────────
            ws_frame = socket.recv() => {
                let ws_msg = match ws_frame {
                    Some(Ok(m)) => m,
                    _ => break, // disconnected or error
                };

                let text = match ws_msg {
                    WsMessage::Text(t) => t.to_string(),
                    WsMessage::Close(_) => break,
                    _ => continue,
                };

                let chat_msg: ChatMsg = match serde_json::from_str(&text) {
                    Ok(m) => m,
                    Err(_) => continue,
                };

                if chat_msg.msg_type != "message" {
                    continue;
                }

                // Send typing indicator immediately
                let _ = socket
                    .send(WsMessage::Text(r#"{"type":"typing"}"#.into()))
                    .await;

                let msg = Message {
                    id: uuid::Uuid::new_v4().to_string(),
                    session_id: own_sid.clone(),
                    channel_id: channel_id.clone(),
                    sender: Sender::User {
                        name: "WebUser".to_string(),
                        id: user_id.clone(),
                    },
                    content: MessageContent::Text(chat_msg.content),
                    timestamp: chrono::Utc::now(),
                };

                // Spawn hub processing in background
                let hub = state.hub.clone();
                let (result_tx, rx) = tokio::sync::oneshot::channel();
                own_result_rx = Some(rx);
                tokio::spawn(async move {
                    let result = hub.handle_message(msg).await;
                    let _ = result_tx.send(result);
                });
            }

            // ── Branch 2: EventBus events (own + cross-channel) ──────────
            event = event_rx.recv() => {
                let event = match event {
                    Ok(e) => e,
                    Err(_) => continue,
                };

                match &event {
                    // --- Own-session events ---------------------------------
                    Event::TextDelta { session_id, text } if *session_id == own_sid => {
                        let json = serde_json::json!({
                            "type": "text_delta",
                            "content": text,
                        });
                        if socket.send(WsMessage::Text(json.to_string().into())).await.is_err() {
                            return;
                        }
                    }
                    Event::ToolInvoked { session_id, tool, .. } if *session_id == own_sid => {
                        let json = serde_json::json!({
                            "type": "tool_invoked",
                            "tool": tool,
                        });
                        let _ = socket.send(WsMessage::Text(json.to_string().into())).await;
                    }
                    Event::ToolResult { session_id, tool, is_error, .. } if *session_id == own_sid => {
                        let json = serde_json::json!({
                            "type": "tool_result",
                            "tool": tool,
                            "is_error": is_error,
                        });
                        let _ = socket.send(WsMessage::Text(json.to_string().into())).await;
                    }

                    // --- Cross-channel: new message from another channel ---
                    Event::MessageReceived(msg) if msg.session_id != own_sid && msg.channel_id.0 != "webchat" => {
                        tracked_foreign.insert(msg.session_id.clone());
                        let sender_name = match &msg.sender {
                            Sender::User { name, .. } => name.clone(),
                            Sender::Agent { agent_id } => agent_id.0.clone(),
                        };
                        let text = match &msg.content {
                            MessageContent::Text(t) => t.clone(),
                            _ => continue,
                        };
                        let json = serde_json::json!({
                            "type": "cross_channel_message",
                            "sessionId": msg.session_id.0,
                            "channel": msg.channel_id.0,
                            "sender": sender_name,
                            "content": text,
                        });
                        let _ = socket.send(WsMessage::Text(json.to_string().into())).await;
                    }

                    // --- Cross-channel: streaming text delta ---------------
                    Event::TextDelta { session_id, text } if tracked_foreign.contains(session_id) => {
                        let json = serde_json::json!({
                            "type": "cross_channel_text_delta",
                            "sessionId": session_id.0,
                            "content": text,
                        });
                        let _ = socket.send(WsMessage::Text(json.to_string().into())).await;
                    }

                    // --- Cross-channel: tool invoked -----------------------
                    Event::ToolInvoked { session_id, tool, .. } if tracked_foreign.contains(session_id) => {
                        let json = serde_json::json!({
                            "type": "cross_channel_tool_invoked",
                            "sessionId": session_id.0,
                            "tool": tool,
                        });
                        let _ = socket.send(WsMessage::Text(json.to_string().into())).await;
                    }

                    // --- Cross-channel: tool result ------------------------
                    Event::ToolResult { session_id, tool, is_error, .. } if tracked_foreign.contains(session_id) => {
                        let json = serde_json::json!({
                            "type": "cross_channel_tool_result",
                            "sessionId": session_id.0,
                            "tool": tool,
                            "is_error": is_error,
                        });
                        let _ = socket.send(WsMessage::Text(json.to_string().into())).await;
                    }

                    // --- Cross-channel: agent response (done) --------------
                    Event::AgentResponse { session_id, content } if tracked_foreign.contains(session_id) => {
                        let json = serde_json::json!({
                            "type": "cross_channel_response",
                            "sessionId": session_id.0,
                            "content": content,
                        });
                        let _ = socket.send(WsMessage::Text(json.to_string().into())).await;
                        tracked_foreign.remove(session_id);
                    }

                    _ => {}
                }
            }

            // ── Branch 3: own-session result ready ───────────────────────
            result = async {
                match own_result_rx.as_mut() {
                    Some(rx) => rx.await,
                    None => std::future::pending().await,
                }
            } => {
                own_result_rx = None;
                let response = match result {
                    Ok(Ok(r)) => r,
                    Ok(Err(e)) => {
                        error!("Agent error: {e}");
                        format!("Error: {e}")
                    }
                    Err(_) => "Internal error".to_string(),
                };
                let reply = ChatMsg {
                    msg_type: "response".to_string(),
                    content: response,
                };
                if let Ok(json) = serde_json::to_string(&reply) {
                    if socket.send(WsMessage::Text(json.into())).await.is_err() {
                        return;
                    }
                }
            }
        }
    }

    info!("WebSocket disconnected: session {own_sid}");
}

// ── API handlers ────────────────────────────────────────────────────────

#[derive(Serialize)]
struct SkillInfo {
    name: String,
    description: String,
    emoji: Option<String>,
    source: String,
    slug: String,
    user_invocable: bool,
}

async fn skills_handler(State(state): State<AppState>) -> Json<Vec<SkillInfo>> {
    let skills: Vec<SkillInfo> = state
        .skills
        .iter()
        .map(|s| SkillInfo {
            name: s.name.clone(),
            description: s.description.clone(),
            emoji: s.metadata.get("emoji").cloned(),
            source: format!("{:?}", s.source),
            slug: s.slug(),
            user_invocable: s.user_invocable,
        })
        .collect();
    Json(skills)
}

#[derive(Serialize)]
struct LlmConfigResponse {
    provider: String,
    model: String,
    auth_mode: String,
    max_iterations: usize,
    has_api_key: bool,
    has_telegram_token: bool,
}

async fn config_get_handler(State(state): State<AppState>) -> Json<LlmConfigResponse> {
    let cfg = state.config.read().await;
    let env_var = match cfg.llm.provider {
        closeclaw_core::config::LlmProvider::Openai => "OPENAI_API_KEY",
        _ => "ANTHROPIC_API_KEY",
    };
    let has_api_key = std::env::var(env_var).map(|v| !v.is_empty()).unwrap_or(false);
    let has_telegram_token = std::env::var("TELOXIDE_TOKEN").map(|v| !v.is_empty()).unwrap_or(false);
    Json(LlmConfigResponse {
        provider: format!("{:?}", cfg.llm.provider).to_lowercase(),
        model: cfg.llm.model.clone(),
        auth_mode: match cfg.llm.auth_mode {
            closeclaw_core::config::AuthMode::ApiKey => "api_key".to_string(),
            closeclaw_core::config::AuthMode::OauthToken => "oauth_token".to_string(),
        },
        max_iterations: cfg.llm.max_iterations,
        has_api_key,
        has_telegram_token,
    })
}

#[derive(Deserialize)]
struct LlmConfigUpdate {
    provider: Option<String>,
    model: Option<String>,
    auth_mode: Option<String>,
    max_iterations: Option<usize>,
    api_key: Option<String>,
    telegram_token: Option<String>,
}

#[derive(Serialize)]
struct ConfigSaveResponse {
    success: bool,
    message: String,
    restart_required: bool,
}

async fn config_post_handler(
    State(state): State<AppState>,
    Json(update): Json<LlmConfigUpdate>,
) -> Result<Json<ConfigSaveResponse>, StatusCode> {
    let mut cfg = state.config.write().await;

    let mut restart_required = false;

    if let Some(ref provider) = update.provider {
        match provider.as_str() {
            "anthropic" => {
                if cfg.llm.provider != closeclaw_core::config::LlmProvider::Anthropic {
                    cfg.llm.provider = closeclaw_core::config::LlmProvider::Anthropic;
                    restart_required = true;
                }
            }
            "openai" => {
                if cfg.llm.provider != closeclaw_core::config::LlmProvider::Openai {
                    cfg.llm.provider = closeclaw_core::config::LlmProvider::Openai;
                    restart_required = true;
                }
            }
            _ => return Err(StatusCode::BAD_REQUEST),
        }
    }

    if let Some(ref model) = update.model {
        if cfg.llm.model != *model {
            cfg.llm.model = model.clone();
            restart_required = true;
        }
    }

    if let Some(ref auth_mode) = update.auth_mode {
        match auth_mode.as_str() {
            "api_key" => {
                if cfg.llm.auth_mode != closeclaw_core::config::AuthMode::ApiKey {
                    cfg.llm.auth_mode = closeclaw_core::config::AuthMode::ApiKey;
                    restart_required = true;
                }
            }
            "oauth_token" => {
                if cfg.llm.auth_mode != closeclaw_core::config::AuthMode::OauthToken {
                    cfg.llm.auth_mode = closeclaw_core::config::AuthMode::OauthToken;
                    restart_required = true;
                }
            }
            _ => return Err(StatusCode::BAD_REQUEST),
        }
    }

    if let Some(max_iter) = update.max_iterations {
        cfg.llm.max_iterations = max_iter;
    }

    // Save actual secrets to ~/.closeclaw/.env and set in process
    let dotenv_dir = std::env::var("HOME").map(PathBuf::from).unwrap_or_else(|_| PathBuf::from(".")).join(".closeclaw");
    let _ = std::fs::create_dir_all(&dotenv_dir);
    let dotenv_path = dotenv_dir.join(".env");
    // Read existing .env lines
    let mut env_map: std::collections::HashMap<String, String> = std::collections::HashMap::new();
    if let Ok(contents) = std::fs::read_to_string(&dotenv_path) {
        for line in contents.lines() {
            if let Some((k, v)) = line.split_once('=') {
                env_map.insert(k.to_string(), v.to_string());
            }
        }
    }
    if let Some(ref api_key) = update.api_key {
        if !api_key.is_empty() {
            let env_var = match cfg.llm.provider {
                closeclaw_core::config::LlmProvider::Openai => "OPENAI_API_KEY",
                _ => "ANTHROPIC_API_KEY",
            };
            env_map.insert(env_var.to_string(), api_key.clone());
            std::env::set_var(env_var, api_key);
            restart_required = true;
        }
    }
    if let Some(ref tg_token) = update.telegram_token {
        if !tg_token.is_empty() {
            env_map.insert("TELOXIDE_TOKEN".to_string(), tg_token.clone());
            std::env::set_var("TELOXIDE_TOKEN", tg_token);
            restart_required = true;
        }
    }
    let env_lines: Vec<String> = env_map.iter().map(|(k, v)| format!("{k}={v}")).collect();
    if !env_lines.is_empty() {
        let _ = std::fs::write(&dotenv_path, env_lines.join("\n") + "\n");
    }

    match cfg.to_toml() {
        Ok(toml_str) => {
            if let Err(e) = std::fs::write(state.config_path.as_ref(), &toml_str) {
                error!("Failed to write config: {e}");
                return Ok(Json(ConfigSaveResponse {
                    success: false,
                    message: format!("Failed to write config file: {e}"),
                    restart_required: false,
                }));
            }
        }
        Err(e) => {
            error!("Failed to serialize config: {e}");
            return Ok(Json(ConfigSaveResponse {
                success: false,
                message: format!("Failed to serialize config: {e}"),
                restart_required: false,
            }));
        }
    }

    let message = if restart_required {
        "Config saved. Restart required for changes to take effect.".to_string()
    } else {
        "Config saved.".to_string()
    };

    Ok(Json(ConfigSaveResponse {
        success: true,
        message,
        restart_required,
    }))
}

// ── HTML SPA ────────────────────────────────────────────────────────────

const CHAT_HTML: &str = r##"<!DOCTYPE html>
<html lang="en" data-theme="dark">
<head>
  <meta charset="UTF-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1.0" />
  <title>CloseClaw</title>
  <link id="hljs-theme" rel="stylesheet" href="https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/styles/github-dark.min.css" />
  <style>
    /* ── Theme variables ─────────────────────────────────────────────── */
    :root, [data-theme="dark"] {
      --bg-body: #1a1a2e;
      --bg-sidebar: #0f0f23;
      --bg-card: #16213e;
      --bg-input: #16213e;
      --bg-code: #0d1b2a;
      --border: #2a2a4a;
      --text-primary: #eee;
      --text-secondary: #aaa;
      --text-muted: #888;
      --text-dim: #555;
      --text-heading: #fff;
      --accent: #e94560;
      --accent-hover: #c73e54;
      --msg-user: #0f3460;
      --msg-agent: #16213e;
      --msg-agent-border: #2a2a4a;
      --link-color: #7dd3fc;
      --code-text: #7dd3fc;
      --code-block-text: #ddd;
      --blockquote-text: #bbb;
      --blockquote-bg: rgba(255,255,255,0.03);
      --table-header-bg: #0f3460;
      --tool-bg: #0f3460;
      --tool-text: #7dd3fc;
      --tool-done-bg: #065f46;
      --tool-done-text: #6ee7b7;
      --badge-source-bg: #533483;
      --badge-source-text: #ddd;
      --badge-slug-bg: #0f3460;
      --badge-slug-text: #7dd3fc;
      --badge-inv-bg: #065f46;
      --badge-inv-text: #6ee7b7;
      --success-bg: #065f46; --success-text: #6ee7b7; --success-border: #10b981;
      --error-bg: #7f1d1d; --error-text: #fca5a5; --error-border: #ef4444;
      --warning-bg: #78350f; --warning-text: #fde68a; --warning-border: #f59e0b;
      --ws-ok-text: #4ade80; --ws-ok-bg: #065f46;
      --ws-err-text: #f87171; --ws-err-bg: #7f1d1d;
    }
    [data-theme="light"] {
      --bg-body: #f0f2f5;
      --bg-sidebar: #ffffff;
      --bg-card: #ffffff;
      --bg-input: #ffffff;
      --bg-code: #f6f8fa;
      --border: #d8dce3;
      --text-primary: #1a1a2e;
      --text-secondary: #555;
      --text-muted: #888;
      --text-dim: #bbb;
      --text-heading: #111;
      --accent: #e94560;
      --accent-hover: #c73e54;
      --msg-user: #dbeafe;
      --msg-agent: #ffffff;
      --msg-agent-border: #d8dce3;
      --link-color: #2563eb;
      --code-text: #d63384;
      --code-block-text: #24292e;
      --blockquote-text: #555;
      --blockquote-bg: rgba(0,0,0,0.03);
      --table-header-bg: #f0f2f5;
      --tool-bg: #dbeafe;
      --tool-text: #1d4ed8;
      --tool-done-bg: #dcfce7;
      --tool-done-text: #15803d;
      --badge-source-bg: #ede9fe;
      --badge-source-text: #6d28d9;
      --badge-slug-bg: #dbeafe;
      --badge-slug-text: #1d4ed8;
      --badge-inv-bg: #dcfce7;
      --badge-inv-text: #15803d;
      --success-bg: #dcfce7; --success-text: #15803d; --success-border: #86efac;
      --error-bg: #fee2e2; --error-text: #b91c1c; --error-border: #fca5a5;
      --warning-bg: #fef3c7; --warning-text: #92400e; --warning-border: #fcd34d;
      --ws-ok-text: #15803d; --ws-ok-bg: #dcfce7;
      --ws-err-text: #b91c1c; --ws-err-bg: #fee2e2;
    }

    /* ── Base ─────────────────────────────────────────────────────────── */
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body { font-family: system-ui, -apple-system, sans-serif; background: var(--bg-body); color: var(--text-primary); display: flex; height: 100vh; overflow: hidden; transition: background 0.2s, color 0.2s; }

    /* ── Sidebar ──────────────────────────────────────────────────────── */
    .sidebar { width: 220px; background: var(--bg-sidebar); display: flex; flex-direction: column; border-right: 1px solid var(--border); flex-shrink: 0; transition: background 0.2s; }
    .sidebar-logo { padding: 1.25rem 1rem; font-size: 1.25rem; font-weight: 700; color: var(--accent); border-bottom: 1px solid var(--border); letter-spacing: 0.5px; display: flex; align-items: center; gap: 0.6rem; }
    .sidebar-logo img { width: 32px; height: 32px; border-radius: 6px; }
    .sidebar-nav { flex: 1; padding: 0.75rem 0; }
    .nav-item { display: flex; align-items: center; gap: 0.75rem; padding: 0.7rem 1rem; cursor: pointer; color: var(--text-secondary); transition: all 0.15s; border-left: 3px solid transparent; font-size: 0.95rem; }
    .nav-item:hover { background: var(--bg-card); color: var(--text-primary); }
    .nav-item.active { background: var(--bg-card); color: var(--accent); border-left-color: var(--accent); }
    .nav-icon { font-size: 1.1rem; width: 1.5rem; text-align: center; }
    .sidebar-footer { padding: 0.75rem 1rem; border-top: 1px solid var(--border); font-size: 0.75rem; color: var(--text-dim); display: flex; align-items: center; justify-content: space-between; }
    .theme-toggle { background: none; border: 1px solid var(--border); border-radius: 6px; padding: 0.3rem 0.5rem; cursor: pointer; font-size: 1rem; line-height: 1; color: var(--text-secondary); transition: all 0.15s; }
    .theme-toggle:hover { border-color: var(--accent); color: var(--accent); }

    /* ── Content ──────────────────────────────────────────────────────── */
    .content { flex: 1; display: flex; flex-direction: column; overflow: hidden; }
    .page { display: none; flex: 1; flex-direction: column; overflow: hidden; }
    .page.active { display: flex; }

    /* ── Chat ─────────────────────────────────────────────────────────── */
    .chat-page { padding: 0; }
    .chat-header { padding: 1rem 1.5rem; border-bottom: 1px solid var(--border); font-size: 1.1rem; font-weight: 600; display: flex; align-items: center; gap: 0.75rem; }
    #messages { flex: 1; overflow-y: auto; padding: 1rem 1.5rem; }
    .msg { margin-bottom: 0.75rem; padding: 0.75rem 1rem; border-radius: 8px; max-width: 85%; line-height: 1.5; transition: background 0.2s; }
    .msg.user { background: var(--msg-user); margin-left: auto; }
    .msg.agent { background: var(--msg-agent); border: 1px solid var(--msg-agent-border); }
    .msg .label { font-size: 0.7rem; opacity: 0.6; margin-bottom: 0.25rem; text-transform: uppercase; letter-spacing: 0.5px; }
    .msg .text { word-wrap: break-word; }
    .msg.user .text { white-space: pre-wrap; }
    .msg.agent .text { line-height: 1.6; }
    .msg.agent .text p { margin-bottom: 0.5em; }
    .msg.agent .text p:last-child { margin-bottom: 0; }
    .msg.agent .text h1, .msg.agent .text h2, .msg.agent .text h3,
    .msg.agent .text h4, .msg.agent .text h5, .msg.agent .text h6 {
      margin: 0.75em 0 0.4em; font-weight: 600; color: var(--text-heading); line-height: 1.3;
    }
    .msg.agent .text h1 { font-size: 1.3em; } .msg.agent .text h2 { font-size: 1.15em; } .msg.agent .text h3 { font-size: 1.05em; }
    .msg.agent .text ul, .msg.agent .text ol { margin: 0.4em 0; padding-left: 1.5em; }
    .msg.agent .text li { margin-bottom: 0.25em; } .msg.agent .text li > p { margin-bottom: 0.2em; }
    .msg.agent .text blockquote { border-left: 3px solid var(--accent); margin: 0.5em 0; padding: 0.25em 0.75em; color: var(--blockquote-text); background: var(--blockquote-bg); border-radius: 0 4px 4px 0; }
    .msg.agent .text code { background: var(--bg-code); padding: 0.15em 0.35em; border-radius: 3px; font-size: 0.88em; font-family: 'SF Mono', 'Fira Code', 'Cascadia Code', monospace; color: var(--code-text); }
    .msg.agent .text pre { background: var(--bg-code); border: 1px solid var(--border); border-radius: 6px; padding: 0.75em 1em; margin: 0.5em 0; overflow-x: auto; }
    .msg.agent .text pre code { background: none; padding: 0; color: var(--code-block-text); font-size: 0.85em; }
    .msg.agent .text table { border-collapse: collapse; margin: 0.5em 0; width: 100%; }
    .msg.agent .text th, .msg.agent .text td { border: 1px solid var(--border); padding: 0.4em 0.6em; text-align: left; font-size: 0.9em; }
    .msg.agent .text th { background: var(--table-header-bg); font-weight: 600; }
    .msg.agent .text hr { border: none; border-top: 1px solid var(--border); margin: 0.75em 0; }
    .msg.agent .text a { color: var(--link-color); text-decoration: none; } .msg.agent .text a:hover { text-decoration: underline; }
    .msg.agent .text img { max-width: 100%; border-radius: 6px; margin: 0.5em 0; }
    .msg.agent .text.streaming { white-space: pre-wrap; }
    .chat-input { display: flex; gap: 0.5rem; padding: 1rem 1.5rem; border-top: 1px solid var(--border); background: var(--bg-sidebar); transition: background 0.2s; }
    .chat-input input { flex: 1; padding: 0.75rem 1rem; border: 1px solid var(--border); border-radius: 8px; background: var(--bg-input); color: var(--text-primary); font-size: 0.95rem; outline: none; transition: background 0.2s, color 0.2s; }
    .chat-input input:focus { border-color: var(--accent); }
    .chat-input button { padding: 0.75rem 1.5rem; border: none; border-radius: 8px; background: var(--accent); color: #fff; font-size: 0.95rem; cursor: pointer; font-weight: 500; }
    .chat-input button:hover { background: var(--accent-hover); }
    .chat-input button:disabled { opacity: 0.5; cursor: not-allowed; }
    #ws-status { font-size: 0.75rem; padding: 0.15rem 0.5rem; border-radius: 4px; }
    .ws-connected { color: var(--ws-ok-text); background: var(--ws-ok-bg); }
    .ws-disconnected { color: var(--ws-err-text); background: var(--ws-err-bg); }

    /* ── Typing ───────────────────────────────────────────────────────── */
    .typing-indicator { display: none; padding: 0.75rem 1rem; margin-bottom: 0.75rem; max-width: 85%; }
    .typing-indicator.visible { display: block; }
    .typing-dots { display: inline-flex; gap: 4px; align-items: center; background: var(--bg-card); border: 1px solid var(--border); border-radius: 8px; padding: 0.6rem 1rem; }
    .typing-dots span { width: 6px; height: 6px; background: var(--accent); border-radius: 50%; animation: bounce 1.2s ease-in-out infinite; }
    .typing-dots span:nth-child(2) { animation-delay: 0.2s; }
    .typing-dots span:nth-child(3) { animation-delay: 0.4s; }
    .typing-label { font-size: 0.7rem; color: var(--text-muted); margin-top: 0.3rem; }
    @keyframes bounce { 0%, 60%, 100% { transform: translateY(0); } 30% { transform: translateY(-6px); } }

    /* ── Tool activity ────────────────────────────────────────────────── */
    .tool-activity { font-size: 0.8rem; color: var(--tool-text); background: var(--tool-bg); border-radius: 6px; padding: 0.4rem 0.75rem; margin-bottom: 0.5rem; max-width: 85%; display: flex; align-items: center; gap: 0.5rem; }
    .tool-activity .spinner { width: 12px; height: 12px; border: 2px solid var(--tool-text); border-top-color: transparent; border-radius: 50%; animation: spin 0.8s linear infinite; }
    @keyframes spin { to { transform: rotate(360deg); } }
    .tool-done { color: var(--tool-done-text); background: var(--tool-done-bg); }

    /* ── Cross-channel messages ──────────────────────────────────────── */
    .msg.cross-user { background: #0f3d2e; margin-left: 0; opacity: 0.92; }
    [data-theme="light"] .msg.cross-user { background: #d1fae5; }
    .msg.cross-agent { background: var(--msg-agent); border: 1px solid #2d6a4f; opacity: 0.92; }
    [data-theme="light"] .msg.cross-agent { border-color: #86efac; }
    .channel-badge { display: inline-block; font-size: 0.6rem; padding: 0.1rem 0.4rem; border-radius: 3px; background: #065f46; color: #6ee7b7; margin-left: 0.4rem; vertical-align: middle; text-transform: lowercase; font-weight: 600; letter-spacing: 0.3px; }
    [data-theme="light"] .channel-badge { background: #dcfce7; color: #15803d; }
    .cross-tool-activity { font-size: 0.8rem; color: #6ee7b7; background: #064e3b; border-radius: 6px; padding: 0.4rem 0.75rem; margin-bottom: 0.5rem; max-width: 85%; display: flex; align-items: center; gap: 0.5rem; opacity: 0.92; }
    [data-theme="light"] .cross-tool-activity { color: #15803d; background: #dcfce7; }
    .cross-tool-activity .spinner { width: 12px; height: 12px; border: 2px solid #6ee7b7; border-top-color: transparent; border-radius: 50%; animation: spin 0.8s linear infinite; }
    [data-theme="light"] .cross-tool-activity .spinner { border-color: #15803d; border-top-color: transparent; }
    .cross-tool-activity.tool-done { color: var(--tool-done-text); background: var(--tool-done-bg); }

    /* ── Lightbox ─────────────────────────────────────────────────────── */
    .lightbox { position: fixed; inset: 0; background: rgba(0,0,0,0.85); display: flex; align-items: center; justify-content: center; z-index: 1000; cursor: pointer; }
    .lightbox img { max-width: 95vw; max-height: 95vh; border-radius: 8px; }
    .msg img { cursor: pointer; }

    /* ── Skills ────────────────────────────────────────────────────────── */
    .skills-page { padding: 1.5rem; overflow-y: auto; }
    .skills-header { font-size: 1.1rem; font-weight: 600; margin-bottom: 1rem; }
    .skills-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 1rem; }
    .skill-card { background: var(--bg-card); border: 1px solid var(--border); border-radius: 10px; padding: 1.25rem; transition: border-color 0.15s, background 0.2s; }
    .skill-card:hover { border-color: var(--accent); }
    .skill-card-header { display: flex; align-items: center; gap: 0.5rem; margin-bottom: 0.5rem; }
    .skill-emoji { font-size: 1.5rem; } .skill-name { font-weight: 600; font-size: 1rem; }
    .skill-desc { color: var(--text-secondary); font-size: 0.85rem; line-height: 1.4; margin-bottom: 0.75rem; }
    .skill-badges { display: flex; gap: 0.5rem; flex-wrap: wrap; }
    .badge { font-size: 0.7rem; padding: 0.2rem 0.5rem; border-radius: 4px; font-family: monospace; }
    .badge-source { background: var(--badge-source-bg); color: var(--badge-source-text); }
    .badge-slug { background: var(--badge-slug-bg); color: var(--badge-slug-text); }
    .badge-invocable { background: var(--badge-inv-bg); color: var(--badge-inv-text); }
    .skills-empty { color: var(--text-dim); font-style: italic; padding: 2rem 0; }

    /* ── Config ────────────────────────────────────────────────────────── */
    .config-page { padding: 1.5rem; overflow-y: auto; }
    .config-header { font-size: 1.1rem; font-weight: 600; margin-bottom: 1rem; }
    .config-form { max-width: 500px; }
    .form-group { margin-bottom: 1.25rem; }
    .form-group label { display: block; font-size: 0.85rem; color: var(--text-secondary); margin-bottom: 0.35rem; font-weight: 500; }
    .form-group select, .form-group input { width: 100%; padding: 0.65rem 0.75rem; border: 1px solid var(--border); border-radius: 6px; background: var(--bg-input); color: var(--text-primary); font-size: 0.9rem; outline: none; transition: background 0.2s, color 0.2s; }
    .form-group select:focus, .form-group input:focus { border-color: var(--accent); }
    .config-save-btn { padding: 0.7rem 1.5rem; border: none; border-radius: 8px; background: var(--accent); color: #fff; font-size: 0.9rem; cursor: pointer; font-weight: 500; }
    .config-save-btn:hover { background: var(--accent-hover); }
    .config-save-btn:disabled { opacity: 0.5; cursor: not-allowed; }
    .config-notice { margin-top: 1rem; padding: 0.75rem 1rem; border-radius: 6px; font-size: 0.85rem; display: none; }
    .config-notice.success { display: block; background: var(--success-bg); color: var(--success-text); border: 1px solid var(--success-border); }
    .config-notice.error { display: block; background: var(--error-bg); color: var(--error-text); border: 1px solid var(--error-border); }
    .config-notice.warning { display: block; background: var(--warning-bg); color: var(--warning-text); border: 1px solid var(--warning-border); }
  </style>
</head>
<body>
  <div class="sidebar">
    <div class="sidebar-logo"><img src="/logo.png" alt="CloseClaw" />CloseClaw</div>
    <nav class="sidebar-nav">
      <div class="nav-item active" data-page="chat">
        <span class="nav-icon">&#128172;</span> Chat
      </div>
      <div class="nav-item" data-page="skills">
        <span class="nav-icon">&#9889;</span> Skills
      </div>
      <div class="nav-item" data-page="config">
        <span class="nav-icon">&#9881;</span> Config
      </div>
    </nav>
    <div class="sidebar-footer">
      <span>CloseClaw</span>
      <button class="theme-toggle" id="theme-toggle" title="Toggle theme">&#127769;</button>
    </div>
  </div>

  <div class="content">
    <div id="page-chat" class="page chat-page active">
      <div class="chat-header">
        Chat
        <span id="ws-status" class="ws-disconnected">disconnected</span>
      </div>
      <div id="messages">
        <div id="typing-indicator" class="typing-indicator">
          <div class="typing-dots"><span></span><span></span><span></span></div>
          <div class="typing-label" id="typing-label">Thinking...</div>
        </div>
      </div>
      <div class="chat-input">
        <input id="input" placeholder="Type a message..." autofocus />
        <button id="send-btn" onclick="sendMsg()">Send</button>
      </div>
    </div>

    <div id="page-skills" class="page skills-page">
      <div class="skills-header">Loaded Skills</div>
      <div id="skills-grid" class="skills-grid">
        <div class="skills-empty">Loading skills...</div>
      </div>
    </div>

    <div id="page-config" class="page config-page">
      <div class="config-header">LLM Configuration</div>
      <div class="config-form">
        <div class="form-group">
          <label>Provider</label>
          <select id="cfg-provider">
            <option value="anthropic">Anthropic</option>
            <option value="openai">OpenAI</option>
          </select>
        </div>
        <div class="form-group">
          <label>Model</label>
          <select id="cfg-model"></select>
        </div>
        <div class="form-group">
          <label>Auth Mode</label>
          <select id="cfg-auth-mode"></select>
        </div>
        <div class="form-group" id="fg-api-key" style="display:none;">
          <label>API Key <span id="api-key-status" style="font-size:0.8em;"></span></label>
          <input id="cfg-api-key" type="password" placeholder="Enter your API key" autocomplete="off" />
        </div>
        <div class="form-group">
          <label>Max Iterations</label>
          <input id="cfg-max-iter" type="number" min="1" max="100" />
        </div>
        <div class="config-header" style="margin-top:1.5rem;">Telegram Configuration</div>
        <div class="form-group">
          <label>Bot Token <span id="tg-token-status" style="font-size:0.8em;"></span></label>
          <input id="cfg-tg-token" type="password" placeholder="Enter your Telegram bot token" autocomplete="off" />
        </div>
        <button class="config-save-btn" onclick="saveConfig()">Save Config</button>
        <div id="config-notice" class="config-notice"></div>
      </div>
    </div>
  </div>

  <script src="https://cdnjs.cloudflare.com/ajax/libs/marked/12.0.2/marked.min.js"></script>
  <script src="https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/highlight.min.js"></script>
  <script>
    // ── Theme ────────────────────────────────────────────────────────────
    const HLJS_DARK = 'https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/styles/github-dark.min.css';
    const HLJS_LIGHT = 'https://cdnjs.cloudflare.com/ajax/libs/highlight.js/11.9.0/styles/github.min.css';

    function getTheme() {
      return localStorage.getItem('closeclaw-theme') || 'dark';
    }
    function applyTheme(theme) {
      document.documentElement.setAttribute('data-theme', theme);
      document.getElementById('hljs-theme').href = theme === 'dark' ? HLJS_DARK : HLJS_LIGHT;
      document.getElementById('theme-toggle').innerHTML = theme === 'dark' ? '&#127769;' : '&#9728;&#65039;';
      localStorage.setItem('closeclaw-theme', theme);
    }
    applyTheme(getTheme());

    document.getElementById('theme-toggle').addEventListener('click', () => {
      applyTheme(getTheme() === 'dark' ? 'light' : 'dark');
    });

    // ── Workspace path (injected by server) ──────────────────────────
    const WORKSPACE = '__WORKSPACE__';

    // ── Markdown setup ──────────────────────────────────────────────────
    marked.setOptions({
      highlight: function(code, lang) {
        if (lang && hljs.getLanguage(lang)) {
          return hljs.highlight(code, { language: lang }).value;
        }
        return hljs.highlightAuto(code).value;
      },
      breaks: true,
      gfm: true,
    });
    function toFilesUrl(path) {
      // Convert an absolute or ~/ image path to a /files/ URL
      if (path.startsWith('/files/')) return null; // already rewritten
      if (path.startsWith('~/')) return '/files/' + path;
      if (path.startsWith('/')) return '/files' + path;
      return null;
    }
    const IMG_EXT = /\.(?:png|jpe?g|gif|webp|svg)$/i;

    function renderMarkdown(text) {
      // Pre-process: convert bare image paths (absolute or ~/...) to markdown images
      text = text.replace(
        /(^|[\s(>])((?:~\/|\/)([\w.\-]+\/)*[\w.\-]+\.(?:png|jpe?g|gif|webp|svg))\b/gim,
        (match, prefix, path) => {
          const url = toFilesUrl(path);
          if (url) {
            const name = path.split('/').pop();
            return prefix + '![' + name + '](' + url + ')';
          }
          return match;
        }
      );
      let html = marked.parse(text);
      // Post-process: rewrite <img src="..."> with absolute/~ paths
      html = html.replace(
        /(<img\s[^>]*src=")((?:~\/|\/)[^"]*\.(?:png|jpe?g|gif|webp|svg))(")/gi,
        (match, pre, src, post) => {
          const url = toFilesUrl(src);
          return url ? pre + url + post : match;
        }
      );
      // Post-process: convert <code>/path/to/image.ext</code> into inline <img>
      html = html.replace(
        /<code>((?:~\/|\/)[^<]*\.(?:png|jpe?g|gif|webp|svg))<\/code>/gi,
        (match, path) => {
          const url = toFilesUrl(path);
          if (url) {
            const name = path.split('/').pop();
            return '<img src="' + url + '" alt="' + name + '" />';
          }
          return match;
        }
      );
      return html;
    }

    // ── Lightbox ─────────────────────────────────────────────────────────
    document.addEventListener('click', e => {
      if (e.target.matches('.msg img')) {
        const lb = document.createElement('div');
        lb.className = 'lightbox';
        lb.innerHTML = '<img src="' + e.target.src + '">';
        lb.onclick = () => lb.remove();
        document.body.appendChild(lb);
      }
    });

    // ── Navigation ──────────────────────────────────────────────────────
    const navItems = document.querySelectorAll('.nav-item');
    const pages = document.querySelectorAll('.page');
    navItems.forEach(item => {
      item.addEventListener('click', () => {
        navItems.forEach(n => n.classList.remove('active'));
        pages.forEach(p => p.classList.remove('active'));
        item.classList.add('active');
        document.getElementById('page-' + item.dataset.page).classList.add('active');
        if (item.dataset.page === 'skills') loadSkills();
        if (item.dataset.page === 'config') loadConfig();
      });
    });

    // ── WebSocket Chat ──────────────────────────────────────────────────
    const messagesEl = document.getElementById('messages');
    const inputEl = document.getElementById('input');
    const sendBtn = document.getElementById('send-btn');
    const wsStatus = document.getElementById('ws-status');
    const typingIndicator = document.getElementById('typing-indicator');
    const typingLabel = document.getElementById('typing-label');
    let ws, reconnectTimer, isProcessing = false, streamingMsgEl = null;
    // Track cross-channel streaming elements keyed by sessionId
    const crossStreamEls = {};

    function connectWs() {
      const proto = location.protocol === 'https:' ? 'wss:' : 'ws:';
      ws = new WebSocket(`${proto}//${location.host}/ws`);
      ws.onopen = () => { wsStatus.textContent = 'connected'; wsStatus.className = 'ws-connected'; };
      ws.onclose = () => { wsStatus.textContent = 'disconnected'; wsStatus.className = 'ws-disconnected'; clearTimeout(reconnectTimer); reconnectTimer = setTimeout(connectWs, 2000); };
      ws.onerror = () => { ws.close(); };
      ws.onmessage = (e) => {
        const data = JSON.parse(e.data);
        switch (data.type) {
          case 'typing': showTyping('Thinking...'); break;
          case 'text_delta': hideTyping(); appendStreamDelta(data.content); break;
          case 'tool_invoked': showToolActivity(data.tool); break;
          case 'tool_result': markToolDone(data.tool, data.is_error); break;
          case 'response': finishResponse(data.content); break;
          case 'cross_channel_message': addCrossChannelMsg(data); break;
          case 'cross_channel_text_delta': appendCrossChannelStreamDelta(data); break;
          case 'cross_channel_tool_invoked': showCrossChannelToolActivity(data); break;
          case 'cross_channel_tool_result': markCrossChannelToolDone(data); break;
          case 'cross_channel_response': finishCrossChannelResponse(data); break;
        }
      };
    }
    connectWs();

    function showTyping(label) { typingLabel.textContent = label; typingIndicator.classList.add('visible'); scrollToBottom(); }
    function hideTyping() { typingIndicator.classList.remove('visible'); }

    function showToolActivity(toolName) {
      hideTyping(); finalizeStreamingMsg();
      const el = document.createElement('div');
      el.className = 'tool-activity'; el.dataset.tool = toolName;
      el.innerHTML = '<div class="spinner"></div> Using ' + esc(toolName) + '...';
      messagesEl.insertBefore(el, typingIndicator);
      showTyping('Working...');
    }
    function markToolDone(toolName, isError) {
      const els = messagesEl.querySelectorAll('.tool-activity[data-tool="' + toolName + '"]');
      const el = els[els.length - 1];
      if (el) { el.classList.add('tool-done'); el.innerHTML = (isError ? '&#10060; ' : '&#10003; ') + esc(toolName) + ' done'; }
    }

    function appendStreamDelta(text) {
      if (!streamingMsgEl) {
        streamingMsgEl = document.createElement('div');
        streamingMsgEl.className = 'msg agent';
        streamingMsgEl.innerHTML = '<div class="label">agent</div><div class="text streaming"></div>';
        messagesEl.insertBefore(streamingMsgEl, typingIndicator);
      }
      streamingMsgEl.querySelector('.text').textContent += text;
      scrollToBottom();
    }
    function finalizeStreamingMsg() { streamingMsgEl = null; }

    // ── Cross-channel functions ──────────────────────────────────────────
    function addCrossChannelMsg(data) {
      const d = document.createElement('div'); d.className = 'msg cross-user';
      d.innerHTML = '<div class="label">' + esc(data.sender) + '<span class="channel-badge">' + esc(data.channel) + '</span></div><div class="text"></div>';
      d.querySelector('.text').textContent = data.content;
      messagesEl.insertBefore(d, typingIndicator); scrollToBottom();
    }

    function appendCrossChannelStreamDelta(data) {
      const sid = data.sessionId;
      if (!crossStreamEls[sid]) {
        const d = document.createElement('div'); d.className = 'msg cross-agent agent';
        d.innerHTML = '<div class="label">agent<span class="channel-badge">cross-channel</span></div><div class="text streaming"></div>';
        messagesEl.insertBefore(d, typingIndicator);
        crossStreamEls[sid] = d;
      }
      crossStreamEls[sid].querySelector('.text').textContent += data.content;
      scrollToBottom();
    }

    function showCrossChannelToolActivity(data) {
      // Finalize any in-progress cross-channel streaming for this session
      if (crossStreamEls[data.sessionId]) {
        crossStreamEls[data.sessionId] = null;
      }
      const el = document.createElement('div');
      el.className = 'cross-tool-activity'; el.dataset.tool = data.tool; el.dataset.sid = data.sessionId;
      el.innerHTML = '<div class="spinner"></div> [' + esc(data.sessionId.slice(0,8)) + '] Using ' + esc(data.tool) + '...';
      messagesEl.insertBefore(el, typingIndicator); scrollToBottom();
    }

    function markCrossChannelToolDone(data) {
      const els = messagesEl.querySelectorAll('.cross-tool-activity[data-tool="' + data.tool + '"][data-sid="' + data.sessionId + '"]');
      const el = els[els.length - 1];
      if (el) { el.classList.add('tool-done'); el.innerHTML = (data.is_error ? '&#10060; ' : '&#10003; ') + '[' + esc(data.sessionId.slice(0,8)) + '] ' + esc(data.tool) + ' done'; }
    }

    function finishCrossChannelResponse(data) {
      const sid = data.sessionId;
      if (crossStreamEls[sid]) {
        const t = crossStreamEls[sid].querySelector('.text');
        t.classList.remove('streaming'); t.innerHTML = renderMarkdown(data.content);
        delete crossStreamEls[sid];
      } else {
        const d = document.createElement('div'); d.className = 'msg cross-agent agent';
        d.innerHTML = '<div class="label">agent<span class="channel-badge">cross-channel</span></div><div class="text"></div>';
        d.querySelector('.text').innerHTML = renderMarkdown(data.content);
        messagesEl.insertBefore(d, typingIndicator);
      }
      scrollToBottom();
    }

    function finishResponse(fullText) {
      hideTyping();
      if (streamingMsgEl) {
        const t = streamingMsgEl.querySelector('.text');
        t.classList.remove('streaming'); t.innerHTML = renderMarkdown(fullText);
        finalizeStreamingMsg();
      } else { addAgentMsg(fullText); }
      isProcessing = false; sendBtn.disabled = false; inputEl.disabled = false; inputEl.focus(); scrollToBottom();
    }

    function addAgentMsg(text) {
      const d = document.createElement('div'); d.className = 'msg agent';
      d.innerHTML = '<div class="label">agent</div><div class="text"></div>';
      d.querySelector('.text').innerHTML = renderMarkdown(text);
      messagesEl.insertBefore(d, typingIndicator); scrollToBottom();
    }
    function addMsg(role, text) {
      if (role === 'agent') { addAgentMsg(text); return; }
      const d = document.createElement('div'); d.className = 'msg ' + role;
      d.innerHTML = '<div class="label">' + role + '</div><div class="text"></div>';
      d.querySelector('.text').textContent = text;
      messagesEl.insertBefore(d, typingIndicator); scrollToBottom();
    }
    function scrollToBottom() { messagesEl.scrollTop = messagesEl.scrollHeight; }

    function sendMsg() {
      const text = inputEl.value.trim();
      if (!text || !ws || ws.readyState !== WebSocket.OPEN || isProcessing) return;
      isProcessing = true; sendBtn.disabled = true; inputEl.disabled = true;
      addMsg('user', text);
      ws.send(JSON.stringify({ type: 'message', content: text }));
      inputEl.value = '';
    }
    inputEl.addEventListener('keydown', (e) => { if (e.key === 'Enter') sendMsg(); });

    // ── Skills ──────────────────────────────────────────────────────────
    let skillsLoaded = false;
    async function loadSkills() {
      if (skillsLoaded) return;
      try {
        const res = await fetch('/api/skills'); const skills = await res.json();
        const grid = document.getElementById('skills-grid');
        if (skills.length === 0) { grid.innerHTML = '<div class="skills-empty">No skills loaded.</div>'; skillsLoaded = true; return; }
        grid.innerHTML = '';
        for (const s of skills) {
          const card = document.createElement('div'); card.className = 'skill-card';
          const emoji = s.emoji || '&#128736;';
          let badges = '<span class="badge badge-source">' + esc(s.source) + '</span><span class="badge badge-slug">/' + esc(s.slug) + '</span>';
          if (s.user_invocable) badges += '<span class="badge badge-invocable">user-invocable</span>';
          card.innerHTML = '<div class="skill-card-header"><span class="skill-emoji">' + emoji + '</span><span class="skill-name">' + esc(s.name) + '</span></div><div class="skill-desc">' + esc(s.description || 'No description') + '</div><div class="skill-badges">' + badges + '</div>';
          grid.appendChild(card);
        }
        skillsLoaded = true;
      } catch (e) { document.getElementById('skills-grid').innerHTML = '<div class="skills-empty">Failed to load skills.</div>'; }
    }

    // ── Config ──────────────────────────────────────────────────────────
    const MODELS = {
      anthropic: [
        { value: 'claude-sonnet-4-20250514', label: 'Claude Sonnet 4' },
        { value: 'claude-opus-4-20250514', label: 'Claude Opus 4' },
        { value: 'claude-haiku-4-5-20251001', label: 'Claude Haiku 4.5' },
      ],
      openai: [
        { value: 'gpt-4o', label: 'GPT-4o' },
        { value: 'gpt-4o-mini', label: 'GPT-4o Mini' },
        { value: 'gpt-4-turbo', label: 'GPT-4 Turbo' },
      ]
    };
    const AUTH_MODES = {
      anthropic: [
        { value: 'api_key', label: 'API Key' },
        { value: 'oauth_token', label: 'OAuth Token (Claude Subscription)' },
      ],
      openai: [
        { value: 'api_key', label: 'API Key' },
      ]
    };

    let configLoaded = false;

    function populateModels(provider, currentModel) {
      const sel = document.getElementById('cfg-model');
      sel.innerHTML = '';
      (MODELS[provider] || []).forEach(m => {
        const opt = document.createElement('option');
        opt.value = m.value; opt.textContent = m.label;
        sel.appendChild(opt);
      });
      if (currentModel) sel.value = currentModel;
    }

    function populateAuthModes(provider, currentMode) {
      const sel = document.getElementById('cfg-auth-mode');
      sel.innerHTML = '';
      (AUTH_MODES[provider] || []).forEach(m => {
        const opt = document.createElement('option');
        opt.value = m.value; opt.textContent = m.label;
        sel.appendChild(opt);
      });
      if (currentMode) sel.value = currentMode;
      toggleAuthFields();
    }

    function toggleAuthFields() {
      const mode = document.getElementById('cfg-auth-mode').value;
      document.getElementById('fg-api-key').style.display = mode === 'api_key' ? '' : 'none';
    }

    document.getElementById('cfg-provider').addEventListener('change', () => {
      const provider = document.getElementById('cfg-provider').value;
      populateModels(provider);
      populateAuthModes(provider);
    });
    document.getElementById('cfg-auth-mode').addEventListener('change', toggleAuthFields);

    async function loadConfig() {
      if (configLoaded) return;
      try {
        const res = await fetch('/api/config'); const cfg = await res.json();
        document.getElementById('cfg-provider').value = cfg.provider;
        populateModels(cfg.provider, cfg.model);
        populateAuthModes(cfg.provider, cfg.auth_mode);
        document.getElementById('cfg-max-iter').value = cfg.max_iterations;
        if (cfg.has_api_key) {
          document.getElementById('api-key-status').textContent = '(configured)';
          document.getElementById('api-key-status').style.color = 'var(--tool-done-text)';
          document.getElementById('cfg-api-key').placeholder = '••••••••  (leave blank to keep current)';
        }
        if (cfg.has_telegram_token) {
          document.getElementById('tg-token-status').textContent = '(configured)';
          document.getElementById('tg-token-status').style.color = 'var(--tool-done-text)';
          document.getElementById('cfg-tg-token').placeholder = '••••••••  (leave blank to keep current)';
        }
        configLoaded = true;
      } catch (e) { showNotice('error', 'Failed to load config.'); }
    }

    async function saveConfig() {
      const notice = document.getElementById('config-notice'); notice.className = 'config-notice'; notice.textContent = '';
      try {
        const body = {
          provider: document.getElementById('cfg-provider').value,
          model: document.getElementById('cfg-model').value,
          auth_mode: document.getElementById('cfg-auth-mode').value,
          max_iterations: parseInt(document.getElementById('cfg-max-iter').value, 10),
        };
        const apiKey = document.getElementById('cfg-api-key').value;
        const tgToken = document.getElementById('cfg-tg-token').value;
        if (apiKey) body.api_key = apiKey;
        if (tgToken) body.telegram_token = tgToken;
        const res = await fetch('/api/config', { method: 'POST', headers: { 'Content-Type': 'application/json' }, body: JSON.stringify(body) });
        const result = await res.json();
        if (result.success) {
          showNotice(result.restart_required ? 'warning' : 'success', result.message);
          // Clear password fields and reload status
          document.getElementById('cfg-api-key').value = '';
          document.getElementById('cfg-tg-token').value = '';
          configLoaded = false;
          loadConfig();
        } else { showNotice('error', result.message); }
      } catch (e) { showNotice('error', 'Failed to save config.'); }
    }

    function showNotice(type, msg) { const el = document.getElementById('config-notice'); el.className = 'config-notice ' + type; el.textContent = msg; }
    function esc(s) { const d = document.createElement('div'); d.textContent = s; return d.innerHTML; }
  </script>
</body>
</html>"##;
