# CloseClaw Architecture

## Overview

CloseClaw is a multi-channel AI agent framework built in Rust. It connects LLMs (Anthropic Claude, OpenAI) to real-world tools and exposes them through multiple channels — a web UI, Telegram, and a terminal REPL — all running simultaneously.

```
                          ┌──────────────────────┐
                          │     LLM Provider     │
                          │  (Anthropic / OpenAI) │
                          └──────────┬───────────┘
                                     │
                          ┌──────────▼───────────┐
                          │    Agent Runtime     │
                          │  (ReAct loop, skills, │
                          │   context building)   │
                          └──────────┬───────────┘
                                     │
               ┌─────────────────────▼─────────────────────┐
               │                   Hub                     │
               │  (Router, Sessions, EventBus, Scheduler)  │
               └────┬──────────┬──────────┬────────────────┘
                    │          │          │
             ┌──────▼───┐ ┌───▼────┐ ┌───▼──────┐
             │  WebChat  │ │Telegram│ │Scheduler │
             │ (Axum+WS) │ │ (Bot)  │ │  (Cron)  │
             └───────────┘ └────────┘ └──────────┘
```

## Crate Map

| Crate | Path | Role |
|---|---|---|
| **closeclaw-core** | `crates/closeclaw-core/` | Foundational traits (`Tool`, `Agent`), types (`Event`, `Message`, `Session`), config parsing, error types |
| **closeclaw-agent** | `crates/closeclaw-agent/` | LLM integration (Anthropic + OpenAI), ReAct loop, tool dispatch, skill loading |
| **closeclaw-gateway** | `crates/closeclaw-gateway/` | Hub (message routing), Router (session mapping), EventBus (broadcast), Scheduler (cron), persistence |
| **closeclaw-channels** | `crates/closeclaw-channels/` | WebChat (Axum HTTP + WebSocket), Telegram (teloxide long polling), CLI (stdin/stdout) |
| **closeclaw-tools** | `crates/closeclaw-tools/` | All built-in tool implementations |
| **closeclaw** | `crates/closeclaw/` | Binary entry point: CLI arg parsing, setup wizard, startup orchestration |

## Message Flow

When a user sends a message through any channel, this is what happens:

```
User (WebChat/Telegram/CLI)
  │
  ▼
Channel layer creates a Message { session_id, channel_id, sender, content }
  │
  ▼
Hub.handle_message(msg)
  ├─ Router.resolve(channel_id, peer_id)
  │    → returns (agent_id, session_id, is_new)
  │    → creates new Session if needed
  │
  ├─ Publishes Event::MessageReceived on EventBus
  │
  ├─ Looks up AgentRuntime for the agent_id
  │
  └─ Calls agent.handle_message(session, user_text, event_tx)
       │
       ▼
     Agent Runtime (ReAct loop)
       ├─ Builds system prompt (SOUL.md + skills + date/time)
       ├─ Appends user text to session history
       └─ Loop (up to max_iterations):
            ├─ Calls LLM with chat history + tool definitions
            ├─ If LLM returns Text → done, return response
            └─ If LLM returns ToolUse:
                 ├─ Emit Event::ToolInvoked
                 ├─ Execute tool via ToolRegistry
                 ├─ Emit Event::ToolResult
                 ├─ Append to history
                 └─ Continue loop
       │
       ▼
Hub publishes Event::AgentResponse
  │
  ▼
Channel layer delivers response to user
```

## Event Bus

All activity flows through a tokio broadcast channel (`EventBus`, capacity 1024). Every WebSocket client subscribes to this bus, enabling:

- **Streaming**: `TextDelta` events carry partial LLM output in real time.
- **Tool activity**: `ToolInvoked` and `ToolResult` events show tool usage live.
- **Cross-channel visibility**: WebChat clients see Telegram conversations and vice versa.
- **System notices**: `SystemNotice` events broadcast server-wide messages (e.g., post-restart).

Event types:

| Event | Description |
|---|---|
| `MessageReceived` | A user sent a message |
| `AgentResponse` | The agent finished responding |
| `ToolInvoked` | A tool was called |
| `ToolResult` | A tool returned a result |
| `TextDelta` | Streaming text chunk from the LLM |
| `SessionCreated` | A new session was created |
| `SessionReset` | A session was cleared |
| `SystemNotice` | Server-wide broadcast (e.g., restart notification) |
| `Error` | An error occurred |

## Session Management

- **Router** maps `(channel_id, peer_id)` → `(agent_id, session_id)`. Each unique user on each channel gets a persistent session.
- **Session** holds the full `ChatMessage` history (System, User, Assistant, ToolUse, ToolResult).
- **SessionStore** persists history as JSONL files at `~/.closeclaw/sessions/{session_id}.jsonl`.
- **Scheduler sessions** use deterministic IDs (`sched-{schedule_id}`) so they persist across restarts.

## Scheduler

The scheduler runs cron-based tasks. Each schedule entry fires a message to the agent on its cron schedule, with its own persistent session.

```toml
[[schedules]]
id = "morning-news"
cron = "0 0 9 * * 1-5 *"     # weekdays at 9:00 AM
message = "Give me a brief news summary."
```

The scheduler can also be managed dynamically at runtime via the `add_schedule`, `remove_schedule`, and `list_schedules` tools. Dynamic schedules are persisted in `~/.closeclaw/schedules.json`.

When a schedule fires with a `notify_peer_id` (auto-captured from the user who created it), the response is delivered back to their Telegram chat via `TelegramNotifier`.

## LLM Integration

Two providers are supported:

### Anthropic

- **API Key mode**: Standard `x-api-key` header against `/v1/messages`.
- **OAuth Token mode**: Uses your Claude subscription (Pro/Team/Max). Reads the token from the macOS Keychain automatically (from Claude Code). Adds `anthropic-beta` headers, prefixes tool names with `mcp_`, and includes billing headers.
- **Streaming**: Full SSE support — text deltas are forwarded to the EventBus in real time.

### OpenAI

- Standard OpenAI chat completions API. Tool calls mapped to OpenAI's `tool_calls` format.
- No streaming (falls back to non-streaming).

## Self-Management (Restart)

CloseClaw can restart itself in-place via the `self_manage` tool:

1. Agent calls `self_manage` with `action: "restart"`.
2. Tool returns immediately (so the LLM can reply to the user).
3. After 5 seconds, the process writes a marker file (`~/.closeclaw/restart-pending`) and calls Unix `exec()` — atomically replacing itself with a fresh instance.
4. On startup, the new process detects the marker, deletes it, and broadcasts a `SystemNotice` event after 3 seconds (giving WebSocket clients time to reconnect).
5. WebChat displays "Server has restarted successfully." as a green banner.
