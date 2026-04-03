# Under the Hood

This document explains the internal mechanics of CloseClaw for developers who want to understand, extend, or contribute to the codebase.

---

## The ReAct Loop

The agent uses a ReAct (Reasoning + Acting) loop — the same pattern used by most modern AI agent frameworks. Here's how it works in `crates/closeclaw-agent/src/runtime.rs`:

```
User message arrives
       │
       ▼
Build context (system prompt + history)
       │
       ▼
  ┌───────────────────────────┐
  │   Call LLM (streaming)    │◄──────┐
  └─────────┬─────────────────┘       │
            │                         │
    ┌───────▼────────┐                │
    │  Text response? │── yes ──► Done│
    └───────┬────────┘                │
            │ no (tool calls)         │
            ▼                         │
    Execute each tool call            │
    Append results to history ────────┘
```

Each iteration:

1. **Context building** (`build_context`): Assembles the system prompt by concatenating:
   - Current date and time
   - Workspace files: `SOUL.md`, `AGENTS.md`, `TOOLS.md`, `IDENTITY.md`, `USER.md` (if they exist)
   - Skill summary (XML block listing available `/slash-commands`)
   - Active skill content (if a slash command was invoked)

2. **LLM call** (`chat_stream`): Sends the full message history + tool definitions. Streams text deltas through a channel that feeds the EventBus.

3. **Tool dispatch**: If the LLM returns tool calls instead of text, each call is dispatched through the `ToolRegistry` and results appended to the conversation history. The loop continues.

4. **Termination**: The loop exits when the LLM returns a text response, or after `max_iterations` (default 25) to prevent infinite loops.

---

## Tool System

### The Tool Trait

```rust
// crates/closeclaw-core/src/tool.rs

#[async_trait]
pub trait Tool: Send + Sync {
    fn definition(&self) -> ToolDefinition;
    async fn execute(&self, input: Value) -> Result<ToolResult>;
}
```

- `definition()` returns the tool's name, description, and JSON Schema parameters. These are sent to the LLM as available tools.
- `execute()` receives the LLM's JSON arguments and returns a `ToolResult` (success or error string).

### Tool Registry

`crates/closeclaw-agent/src/tool_dispatch.rs` maintains a `HashMap<String, Arc<dyn Tool>>`. Tools are registered at startup:

```rust
// crates/closeclaw-tools/src/lib.rs
pub fn builtin_tools(workspace: &Path) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(exec::ExecTool::new(workspace.to_path_buf())),
        Arc::new(read_file::ReadFileTool::new(workspace.to_path_buf())),
        // ... all built-in tools
        Arc::new(self_manage::SelfManageTool),
    ]
}
```

### Adding a New Tool

1. Create `crates/closeclaw-tools/src/my_tool.rs`:

```rust
use async_trait::async_trait;
use closeclaw_core::error::Result;
use closeclaw_core::tool::{Tool, ToolDefinition, ToolResult};
use serde_json::{json, Value};

pub struct MyTool;

#[async_trait]
impl Tool for MyTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "my_tool".to_string(),
            description: "What this tool does.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "param1": {
                        "type": "string",
                        "description": "What param1 is for"
                    }
                },
                "required": ["param1"]
            }),
        }
    }

    async fn execute(&self, input: Value) -> Result<ToolResult> {
        let param1 = input["param1"].as_str().ok_or_else(|| {
            closeclaw_core::error::CloseClawError::Tool("missing 'param1'".into())
        })?;
        // Do the work...
        Ok(ToolResult::success(format!("Result: {param1}")))
    }
}
```

2. Register in `crates/closeclaw-tools/src/lib.rs`:

```rust
pub mod my_tool;
// In builtin_tools():
Arc::new(my_tool::MyTool),
```

---

## LLM Providers

### Anthropic Provider

`crates/closeclaw-agent/src/llm.rs` — `AnthropicProvider`

Two auth paths with different behaviors:

| | API Key | OAuth Token |
|---|---|---|
| Header | `x-api-key: <key>` | `Authorization: Bearer <token>` |
| Endpoint | `/v1/messages` | `/v1/messages?beta=true` |
| Tool names | As-is | Prefixed with `mcp_` |
| System prompt | Your `SOUL.md` only | Prepends Claude Code system prefix |
| Billing | None | SHA-256 billing header |

**Streaming** uses Server-Sent Events (SSE). The provider parses these event types:
- `content_block_start` — identifies text or tool_use blocks
- `content_block_delta` — carries `text_delta` or `input_json_delta`
- `content_block_stop` — finalizes the block
- `message_stop` — end of response

Text deltas are sent through a `tokio::mpsc` channel to the agent runtime, which forwards them to the EventBus, which delivers them to WebSocket clients for real-time display.

### OpenAI Provider

Standard OpenAI chat completions API. Messages are converted between CloseClaw's `ChatMessage` format and OpenAI's format:

| ChatMessage | OpenAI role |
|---|---|
| `System(text)` | `system` |
| `User(text)` | `user` |
| `Assistant(text)` | `assistant` |
| `ToolUse` | `assistant` with `tool_calls` |
| `ToolResult` | `tool` |

---

## Hub & Router

### Hub (`crates/closeclaw-gateway/src/hub.rs`)

The Hub is the central coordinator. It holds:

- `agents: DashMap<AgentId, Arc<AgentRuntime>>` — registered agents
- `sessions: DashMap<SessionId, Session>` — active sessions (in-memory)
- `router: Router` — maps (channel, peer) → (agent, session)
- `session_store: SessionStore` — persistence layer
- `event_bus: EventBus` — broadcast channel

Key method: `handle_message(msg)`:
1. Resolve the route (which agent handles this user on this channel)
2. Create or load the session
3. Broadcast the incoming message event
4. Run the agent's ReAct loop
5. Broadcast the response event
6. Return the response text

### Router (`crates/closeclaw-gateway/src/router.rs`)

A `DashMap<RouteKey, (AgentId, SessionId)>` where `RouteKey = (ChannelId, peer_id: String)`.

- **WebChat**: peer_id is a random UUID per WebSocket connection.
- **Telegram**: peer_id is `tg:{chat_id}` — persistent across messages from the same user.
- **Scheduler**: peer_id is `"scheduler"`, seeded via `hub.restore_session()` on startup.

---

## EventBus

`crates/closeclaw-gateway/src/events.rs`

A thin wrapper around `tokio::sync::broadcast::channel<Event>` with capacity 1024.

```rust
pub struct EventBus { tx: broadcast::Sender<Event> }
```

- `publish(event)` — sends to all subscribers
- `subscribe()` — returns a new `broadcast::Receiver<Event>`
- `sender()` — returns a clone of the `Sender` for external use

Every WebSocket connection subscribes on connect. This enables:
- Streaming text deltas to the browser
- Cross-channel visibility (WebChat sees Telegram, etc.)
- System-wide broadcasts (restart notices)

---

## Session Persistence

### In-Memory

`Session` objects live in the Hub's `DashMap`. They hold the full `Vec<ChatMessage>` history.

### On Disk

`SessionStore` (`crates/closeclaw-gateway/src/session_store.rs`) writes each message as a JSON line to `~/.closeclaw/sessions/{session_id}.jsonl`:

```jsonl
{"System":"You are CloseClaw..."}
{"User":"What's the weather?"}
{"Assistant":"Let me check..."}
{"ToolUse":{"id":"call_1","name":"web_search","input":{"query":"weather today"}}}
{"ToolResult":{"id":"call_1","output":"72°F, partly cloudy","is_error":false}}
{"Assistant":"The current weather is 72°F and partly cloudy."}
```

On restart, scheduler sessions are restored via `scheduler.restore_sessions(&hub)` which pre-seeds the router and loads history from disk.

---

## Skill System Internals

### Loading (`crates/closeclaw-agent/src/runtime.rs`)

Skills are loaded from multiple directories, with later directories taking priority:

```
~/.closeclaw/skills/     (index 0: Global)
{workspace}/skills/      (index 1: Workspace)
agent.skills_dir         (index 2: Workspace)
```

For each directory, the loader looks for:
- **Folder-based** (preferred): `{dir}/{slug}/SKILL.md`
- **Legacy flat**: `{dir}/{slug}.md`

The slug is derived from the folder/file name, lowercased, with `_` replaced by `-`.

### Gating

Skills can declare requirements in their frontmatter:

```yaml
metadata:
  requires:
    bins: ["python3", "ffmpeg"]  # required binaries on PATH
    env: ["API_KEY"]             # required env vars
    os: ["macos", "linux"]       # required OS
```

If any requirement isn't met, the skill is silently skipped.

### Injection into System Prompt

All non-disabled skills are listed in an XML summary:

```xml
<available-skills>
  <skill name="News Briefing" slash="/news-briefing" emoji="📰">Search and summarize recent news.</skill>
  <skill name="Code Review" slash="/code-review" emoji="🔍">Review code for bugs and style.</skill>
</available-skills>
```

When a user types `/news-briefing some topic`, the agent detects the slash command, and the full SKILL.md content is injected as an "Active Skill" section in the system prompt for that request.

---

## Self-Management (Restart) Internals

The restart mechanism uses Unix `exec()` for a clean in-place process replacement.

### Timeline

```
T+0s    Tool returns "Restarting in 5 seconds..." to the LLM
T+0-5s  LLM generates farewell message, streams it to the user
T+5s    restart_process() called:
          ├─ Writes ~/.closeclaw/restart-pending marker file
          └─ Calls exec() → process image replaced atomically
T+5s    New process starts (same binary, same args, same PID)
          ├─ Loads config, starts channels
          ├─ Detects marker file, deletes it
          └─ Spawns task: sleep 3s, then broadcast SystemNotice
T+7s    WebSocket clients have reconnected (2s auto-retry)
T+8s    SystemNotice "Server has restarted successfully." delivered
```

### Why exec()?

`exec()` replaces the current process image atomically. This means:
- The listening port is released and immediately available for the new process
- No zombie processes, no orphaned children
- Same PID — OS-level process identity is preserved
- Args and environment are inherited naturally

### The Marker File

The marker file (`~/.closeclaw/restart-pending`) bridges the gap between the old and new process:

1. **Old process** writes it right before `exec()`
2. **New process** checks for it on startup in `run_gateway()`
3. If found: deletes it and schedules a `SystemNotice` event after 3 seconds
4. The 3-second delay gives WebSocket clients time to reconnect (they retry every 2 seconds)

---

## WebChat Frontend

The WebChat UI is a single-page app embedded directly in the Rust binary as a `const` string (`CHAT_HTML` in `webchat.rs`). No separate build step or static files.

### Features

- **WebSocket protocol**: JSON messages with `type` field:
  - Outgoing: `{type: "message", content: "..."}`
  - Incoming: `typing`, `text_delta`, `tool_invoked`, `tool_result`, `response`, `system_notice`, plus cross-channel variants
- **Markdown rendering**: Uses `marked.js` + `highlight.js` for code syntax highlighting
- **Image support**: Detects image paths in responses and renders them inline; lightbox on click
- **Auto-reconnect**: WebSocket reconnects after 2 seconds on disconnect
- **Theme toggle**: Dark/light mode with CSS variables
- **Three tabs**: Chat, Skills (browse loaded skills), Config (live LLM settings editor)

### Config API

The WebChat server exposes REST endpoints for live configuration:

- `GET /api/config` — returns current LLM settings
- `POST /api/config` — updates config, writes `config.toml` and `~/.closeclaw/.env`
- `GET /api/skills` — returns all loaded skills as JSON

---

## Telegram Channel Internals

### Message Processing

`crates/closeclaw-channels/src/telegram.rs` uses `teloxide::repl` for long polling:

1. Receives update from Telegram API
2. Extracts text (or downloads photo/document to `{workspace}/downloads/`)
3. Constructs `Message` with `channel_id: "telegram"`, `peer_id: "tg:{chat_id}"`
4. Sends typing indicator
5. Calls `hub.handle_message(msg)`
6. Extracts any image paths from the response and sends them as Telegram photos
7. Converts remaining markdown to Telegram HTML and sends (split at 4096 chars)

### Markdown to Telegram HTML

The `markdown_to_telegram_html` function converts:

| Markdown | Telegram HTML |
|---|---|
| `**bold**` | `<b>bold</b>` |
| `*italic*` | `<i>italic</i>` |
| `` `code` `` | `<code>code</code>` |
| ```` ```block``` ```` | `<pre>block</pre>` |
| `# Header` | `<b>Header</b>` |
| `[text](url)` | `<a href="url">text</a>` |
| `---` | `———` |

HTML entities (`&`, `<`, `>`) are escaped before conversion.

---

## Error Handling

```rust
// crates/closeclaw-core/src/error.rs
pub enum CloseClawError {
    Tool(String),              // Tool execution failure
    Llm(String),               // LLM API failure
    SessionNotFound(String),   // Unknown session ID
    AgentNotFound(String),     // Unknown agent ID
    Channel(String),           // Channel-level error
    Config(String),            // Configuration error
    Io(std::io::Error),        // File I/O
    Json(serde_json::Error),   // JSON parse error
    MaxIterations(usize),      // ReAct loop exhausted
    Other(String),             // Catch-all
}
```

Tool errors are **non-fatal** — they're returned to the LLM as `ToolResult::error(...)` and the agent can retry or explain the failure to the user. LLM errors and max-iteration errors bubble up and are returned to the channel as error messages.
