use axum::{
    extract::{
        ws::{Message as WsMessage, WebSocket, WebSocketUpgrade},
        State,
    },
    response::Html,
    routing::get,
    Router,
};
use closeclaw_core::types::{
    ChannelId, Message, MessageContent, Sender, SessionId,
};
use closeclaw_gateway::hub::Hub;
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tracing::{error, info};

#[derive(Clone)]
struct AppState {
    hub: Arc<Hub>,
}

#[derive(Serialize, Deserialize)]
struct ChatMsg {
    #[serde(rename = "type")]
    msg_type: String,
    content: String,
}

pub async fn serve(hub: Arc<Hub>, bind: &str, port: u16) -> anyhow::Result<()> {
    let state = AppState { hub };
    let app = Router::new()
        .route("/", get(index_handler))
        .route("/ws", get(ws_handler))
        .with_state(state);

    let addr = format!("{bind}:{port}");
    info!("WebChat listening on {addr}");
    let listener = tokio::net::TcpListener::bind(&addr).await?;
    axum::serve(listener, app).await?;
    Ok(())
}

async fn index_handler() -> Html<&'static str> {
    Html(CHAT_HTML)
}

async fn ws_handler(ws: WebSocketUpgrade, State(state): State<AppState>) -> impl axum::response::IntoResponse {
    ws.on_upgrade(move |socket| handle_ws(socket, state))
}

async fn handle_ws(mut socket: WebSocket, state: AppState) {
    let session_id = SessionId(uuid::Uuid::new_v4().to_string());
    let channel_id = ChannelId("webchat".to_string());
    let user_id = uuid::Uuid::new_v4().to_string();

    info!("WebSocket connected: session {session_id}");

    while let Some(Ok(ws_msg)) = socket.recv().await {
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

        let msg = Message {
            id: uuid::Uuid::new_v4().to_string(),
            session_id: session_id.clone(),
            channel_id: channel_id.clone(),
            sender: Sender::User {
                name: "WebUser".to_string(),
                id: user_id.clone(),
            },
            content: MessageContent::Text(chat_msg.content),
            timestamp: chrono::Utc::now(),
        };

        let response = match state.hub.handle_message(msg).await {
            Ok(r) => r,
            Err(e) => {
                error!("Agent error: {e}");
                format!("Error: {e}")
            }
        };

        let reply = ChatMsg {
            msg_type: "response".to_string(),
            content: response,
        };

        if let Ok(json) = serde_json::to_string(&reply) {
            if socket.send(WsMessage::Text(json.into())).await.is_err() {
                break;
            }
        }
    }

    info!("WebSocket disconnected: session {session_id}");
}

const CHAT_HTML: &str = r#"<!DOCTYPE html>
<html>
<head>
  <title>CloseClaw Chat</title>
  <style>
    * { box-sizing: border-box; margin: 0; padding: 0; }
    body { font-family: system-ui, sans-serif; background: #1a1a2e; color: #eee; display: flex; justify-content: center; padding: 2rem; }
    .chat { max-width: 700px; width: 100%; }
    h1 { margin-bottom: 1rem; color: #e94560; }
    #messages { background: #16213e; border-radius: 8px; padding: 1rem; height: 60vh; overflow-y: auto; margin-bottom: 1rem; }
    .msg { margin-bottom: 0.5rem; padding: 0.5rem; border-radius: 4px; }
    .msg.user { background: #0f3460; }
    .msg.agent { background: #533483; }
    .msg .label { font-size: 0.75rem; opacity: 0.7; margin-bottom: 0.25rem; }
    .input-row { display: flex; gap: 0.5rem; }
    input { flex: 1; padding: 0.75rem; border: none; border-radius: 4px; background: #16213e; color: #eee; font-size: 1rem; }
    button { padding: 0.75rem 1.5rem; border: none; border-radius: 4px; background: #e94560; color: #fff; font-size: 1rem; cursor: pointer; }
    button:hover { background: #c73e54; }
  </style>
</head>
<body>
  <div class="chat">
    <h1>CloseClaw</h1>
    <div id="messages"></div>
    <div class="input-row">
      <input id="input" placeholder="Type a message..." onkeydown="if(event.key==='Enter')send()" autofocus />
      <button onclick="send()">Send</button>
    </div>
  </div>
  <script>
    const ws = new WebSocket(`ws://${location.host}/ws`);
    const messages = document.getElementById('messages');
    const input = document.getElementById('input');

    function addMsg(role, text) {
      const d = document.createElement('div');
      d.className = `msg ${role}`;
      d.innerHTML = `<div class="label">${role}</div><div>${text.replace(/\n/g, '<br>')}</div>`;
      messages.appendChild(d);
      messages.scrollTop = messages.scrollHeight;
    }

    ws.onmessage = (e) => {
      const data = JSON.parse(e.data);
      if (data.type === 'response') addMsg('agent', data.content);
    };

    function send() {
      const text = input.value.trim();
      if (!text) return;
      addMsg('user', text);
      ws.send(JSON.stringify({ type: 'message', content: text }));
      input.value = '';
    }
  </script>
</body>
</html>"#;
