use axum::{
    extract::State,
    response::Html,
    routing::{get, post},
    Json, Router,
};
use serde::Deserialize;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::{oneshot, Mutex};
use tracing::info;

#[derive(Clone)]
struct SetupState {
    config_path: PathBuf,
    shutdown_tx: Arc<Mutex<Option<oneshot::Sender<()>>>>,
}

#[derive(Deserialize)]
struct SetupForm {
    provider: String,
    model: String,
    api_key: String,
    telegram_token: Option<String>,
}

pub async fn serve_setup(bind: &str, port: u16, config_path: PathBuf) -> anyhow::Result<()> {
    let (shutdown_tx, shutdown_rx) = oneshot::channel::<()>();

    let state = SetupState {
        config_path,
        shutdown_tx: Arc::new(Mutex::new(Some(shutdown_tx))),
    };

    let app = Router::new()
        .route("/", get(setup_page))
        .route("/setup", post(handle_setup))
        .with_state(state);

    let addr = format!("{bind}:{port}");
    info!("Setup UI at http://{addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app)
        .with_graceful_shutdown(async { shutdown_rx.await.ok(); })
        .await?;

    Ok(())
}

async fn setup_page() -> Html<&'static str> {
    Html(SETUP_HTML)
}

async fn handle_setup(
    State(state): State<SetupState>,
    Json(form): Json<SetupForm>,
) -> &'static str {
    let env_var = match form.provider.as_str() {
        "openai" => "OPENAI_API_KEY",
        _ => "ANTHROPIC_API_KEY",
    };

    // Set env vars in-process so run_gateway() picks them up
    std::env::set_var(env_var, &form.api_key);
    if let Some(ref token) = &form.telegram_token {
        if !token.is_empty() {
            std::env::set_var("TELOXIDE_TOKEN", token);
        }
    }

    // Persist to ~/.closeclaw/.env
    let dotenv_dir = dirs_home().join(".closeclaw");
    let _ = std::fs::create_dir_all(&dotenv_dir);
    let mut env_lines = vec![format!("{}={}", env_var, form.api_key)];
    if let Some(ref token) = &form.telegram_token {
        if !token.is_empty() {
            env_lines.push(format!("TELOXIDE_TOKEN={token}"));
        }
    }
    let _ = std::fs::write(dotenv_dir.join(".env"), env_lines.join("\n") + "\n");

    // Write config.toml
    let telegram_section = match &form.telegram_token {
        Some(token) if !token.is_empty() => {
            "\n[[channels]]\ntype = \"telegram\"\nenabled = true\ntoken_env = \"TELOXIDE_TOKEN\"\n"
        }
        _ => "",
    };

    let config_content = format!(
        r#"[gateway]
bind = "127.0.0.1"
port = 3000

[[agents]]
id = "default"
name = "CloseClaw Agent"
tools = ["exec", "read_file", "write_file", "web_fetch", "web_search", "list_files", "create_file", "delete_file", "search_files", "browser"]

[[channels]]
type = "webchat"
enabled = true
{telegram_section}
[llm]
provider = "{provider}"
model = "{model}"
auth_mode = "api_key"
api_key_env = "{env_var}"
max_iterations = 25
"#,
        provider = form.provider,
        model = form.model,
        env_var = env_var,
        telegram_section = telegram_section,
    );

    let _ = std::fs::write(&state.config_path, config_content);

    // Signal shutdown so serve_setup() returns
    if let Some(tx) = state.shutdown_tx.lock().await.take() {
        let _ = tx.send(());
    }

    "ok"
}

fn dirs_home() -> PathBuf {
    std::env::var("HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("."))
}

const SETUP_HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
  <title>CloseClaw Setup</title>
  <style>
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body { font-family: system-ui, sans-serif; background: #1a1a2e; color: #eee; display: flex; justify-content: center; align-items: center; min-height: 100vh; padding: 2rem; }
    .setup { max-width: 500px; width: 100%; }
    h1 { margin-bottom: 0.5rem; color: #e94560; }
    p.sub { opacity: 0.7; margin-bottom: 2rem; }
    label { display: block; margin-bottom: 0.25rem; font-size: 0.9rem; opacity: 0.8; margin-top: 1rem; }
    label:first-of-type { margin-top: 0; }
    select, input { width: 100%; padding: 0.75rem; border: none; border-radius: 4px; background: #16213e; color: #eee; font-size: 1rem; }
    select { cursor: pointer; }
    button { width: 100%; padding: 0.75rem; border: none; border-radius: 4px; background: #e94560; color: #fff; font-size: 1rem; cursor: pointer; margin-top: 1.5rem; }
    button:hover { background: #c73e54; }
    button:disabled { opacity: 0.5; cursor: not-allowed; }
    .optional { font-size: 0.75rem; opacity: 0.5; }
    .status { text-align: center; margin-top: 1rem; font-size: 0.9rem; opacity: 0.7; }
  </style>
</head>
<body>
  <div class="setup">
    <h1>CloseClaw</h1>
    <p class="sub">Configure your agent to get started.</p>

    <label>Provider</label>
    <select id="provider" onchange="updateModels()">
      <option value="anthropic">Anthropic</option>
      <option value="openai">OpenAI</option>
    </select>

    <label>Model</label>
    <select id="model"></select>

    <label>API Key</label>
    <input id="api_key" type="password" placeholder="sk-..." />

    <label>Telegram Bot Token <span class="optional">(optional)</span></label>
    <input id="telegram_token" placeholder="123456:ABC-DEF..." />

    <button id="btn" onclick="submit()">Save &amp; Start</button>
    <div id="status" class="status"></div>
  </div>
  <script>
    const models = {
      anthropic: [
        ["claude-sonnet-4-20250514", "Claude Sonnet 4"],
        ["claude-opus-4-20250514", "Claude Opus 4"],
        ["claude-haiku-4-5-20251001", "Claude Haiku 4.5"]
      ],
      openai: [
        ["gpt-4o", "GPT-4o"],
        ["gpt-4o-mini", "GPT-4o Mini"],
        ["gpt-4-turbo", "GPT-4 Turbo"]
      ]
    };

    function updateModels() {
      const sel = document.getElementById("model");
      const prov = document.getElementById("provider").value;
      sel.innerHTML = "";
      for (const [val, label] of models[prov]) {
        const opt = document.createElement("option");
        opt.value = val;
        opt.textContent = label;
        sel.appendChild(opt);
      }
    }
    updateModels();

    async function submit() {
      const btn = document.getElementById("btn");
      const status = document.getElementById("status");
      const apiKey = document.getElementById("api_key").value.trim();
      if (!apiKey) { status.textContent = "API key is required."; return; }

      btn.disabled = true;
      btn.textContent = "Saving...";
      status.textContent = "";

      const body = {
        provider: document.getElementById("provider").value,
        model: document.getElementById("model").value,
        api_key: apiKey,
        telegram_token: document.getElementById("telegram_token").value.trim() || null
      };

      try {
        await fetch("/setup", {
          method: "POST",
          headers: { "Content-Type": "application/json" },
          body: JSON.stringify(body)
        });
      } catch (_) {}

      status.textContent = "Starting agent...";

      const poll = setInterval(async () => {
        try {
          const res = await fetch("/");
          if (res.ok) {
            const text = await res.text();
            if (text.includes("CloseClaw Chat")) {
              clearInterval(poll);
              location.reload();
            }
          }
        } catch (_) {}
      }, 500);
    }
  </script>
</body>
</html>"#;
