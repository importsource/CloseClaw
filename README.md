# CloseClaw

An agent framework in Rust. Build AI agents that use tools, persist sessions, and talk to users over CLI, WebChat, or Telegram.

## Features

- **ReAct agent loop** — iterative reasoning + tool use (up to configurable max iterations)
- **Multi-channel** — CLI, WebChat (WebSocket), Telegram bot
- **Multi-provider** — Anthropic (Claude) and OpenAI
- **Built-in tools** — shell exec, file read/write, web fetch, web search, browser automation
- **Session persistence** — JSONL-based conversation history per user
- **Customizable personality** — SOUL.md system prompt + skill files
- **OAuth support** — compatible with Claude Code's login flow + macOS Keychain

## Quick Start

### 1. Build

```bash
git clone https://github.com/importsource/CloseClaw.git
cd CloseClaw
cargo build --release
```

### 2. Set your API key

```bash
export ANTHROPIC_API_KEY="sk-ant-..."
```

Or for OpenAI:

```bash
export OPENAI_API_KEY="sk-..."
```

### 3. Run

```bash
# Interactive CLI chat (default mode, no config file needed)
./target/release/closeclaw

# With a config file
./target/release/closeclaw --config config.toml chat
```

## Subcommands

| Command | Description |
|---------|-------------|
| `closeclaw chat` | Interactive CLI REPL (default if no subcommand given) |
| `closeclaw run` | Start gateway with all enabled channels (WebChat + Telegram) |
| `closeclaw telegram` | Standalone Telegram bot mode |

**Global flags:**

```
-c, --config <PATH>       Config file path (default: config.toml)
-w, --workspace <PATH>    Workspace directory for tools to operate in
```

## Configuration

Copy and edit the default config:

```bash
cp config.default.toml config.toml
```

```toml
[gateway]
bind = "127.0.0.1"
port = 3000

[[agents]]
id = "default"
name = "My Agent"
tools = ["exec", "read_file", "write_file", "web_fetch", "web_search"]
# soul_md = "SOUL.md"       # System prompt file
# skills_dir = "skills"     # Directory of skill .md files

[[channels]]
type = "cli"
enabled = true

[[channels]]
type = "webchat"
enabled = true

# [[channels]]
# type = "telegram"
# enabled = true
# token_env = "TELOXIDE_TOKEN"

[llm]
provider = "anthropic"                  # "anthropic" or "openai"
model = "claude-sonnet-4-20250514"
auth_mode = "api_key"                   # "api_key" or "oauth_token"
api_key_env = "ANTHROPIC_API_KEY"
max_iterations = 25
# base_url = "https://custom-endpoint.example.com"
```

## Built-in Tools

| Tool | Description |
|------|-------------|
| `exec` | Run shell commands (with optional timeout, default 30s) |
| `read_file` | Read files from the workspace (path-traversal protected) |
| `write_file` | Create or overwrite files in the workspace |
| `web_fetch` | Fetch a URL and convert HTML to text |
| `web_search` | Web search (placeholder — wire up your preferred search API) |
| `browser` | Browser automation via Chrome/Edge CDP + Playwright (see [setup](docs/browser-tool.md)) |

All file tools are sandboxed to the workspace directory.

## Channels

### CLI

The default mode. Type messages at the `> ` prompt, get responses inline.

```bash
closeclaw chat
```

### WebChat

A browser-based chat UI served over HTTP + WebSocket.

```bash
closeclaw run
# Open http://localhost:3000
```

### Telegram

Uses long polling — works behind NAT, no SSL or public URL required.

**Setup:**

1. Talk to [@BotFather](https://t.me/botfather) on Telegram, send `/newbot`, copy the token
2. Set the token:
   ```bash
   export TELOXIDE_TOKEN="123456:ABC-DEF..."
   ```
3. Run:
   ```bash
   # Standalone mode
   closeclaw telegram

   # Or as part of the full gateway (alongside WebChat)
   closeclaw run
   ```

Each Telegram user/group automatically gets a persistent session.

## System Prompt & Skills

Customize your agent by placing markdown files in the workspace:

| File | Purpose |
|------|---------|
| `SOUL.md` | Core identity and instructions |
| `IDENTITY.md` | Personality and traits |
| `AGENTS.md` | Multi-agent definitions |
| `TOOLS.md` | Tool usage guidelines |
| `USER.md` | User-specific context |
| `skills/*.md` | Reusable skill definitions |

All files are optional. They are concatenated into the system prompt for every LLM call.

**Skill file format:**

```markdown
# Skill Name
Brief description of the skill

Detailed instructions, examples, templates, etc.
```

## LLM Providers

### Anthropic (default)

```toml
[llm]
provider = "anthropic"
model = "claude-sonnet-4-20250514"
auth_mode = "api_key"
api_key_env = "ANTHROPIC_API_KEY"
```

**OAuth mode** — use your Claude subscription (Pro/Team/Max) instead of an API key:

1. Install [Claude Code](https://docs.anthropic.com/en/docs/claude-code) and log in:
   ```bash
   npm install -g @anthropic-ai/claude-code
   claude login
   ```
2. Set your config:
   ```toml
   [llm]
   provider = "anthropic"
   model = "claude-sonnet-4-20250514"
   auth_mode = "oauth_token"
   ```

On macOS, CloseClaw automatically reads the OAuth token from the Keychain (where `claude login` stores it). On other platforms, set the token manually:

```bash
export CLAUDE_CODE_TOKEN="sk-ant-oat01-..."
```

Available models with Claude subscription:

| Model | Config value |
|-------|-------------|
| Sonnet 4 | `claude-sonnet-4-20250514` |
| Haiku 4.5 | `claude-haiku-4-5-20251001` |

### OpenAI

```toml
[llm]
provider = "openai"
model = "gpt-4o"
api_key_env = "OPENAI_API_KEY"
```

## Deployment

### Bare metal / VPS

```bash
cargo build --release
export ANTHROPIC_API_KEY="sk-ant-..."
export TELOXIDE_TOKEN="123456:ABC-DEF..."
./target/release/closeclaw run --config config.toml
```

### systemd

```ini
[Unit]
Description=CloseClaw Agent
After=network.target

[Service]
ExecStart=/opt/closeclaw/closeclaw run --config /opt/closeclaw/config.toml
WorkingDirectory=/opt/closeclaw
Environment=ANTHROPIC_API_KEY=sk-ant-...
Environment=TELOXIDE_TOKEN=123456:ABC-DEF...
Restart=always
RestartSec=5

[Install]
WantedBy=multi-user.target
```

```bash
sudo cp closeclaw.service /etc/systemd/system/
sudo systemctl enable --now closeclaw
```

## Documentation

| Guide | Description |
|-------|-------------|
| [Browser Tool](docs/browser-tool.md) | Setup, prerequisites, supported actions, architecture |
| [LinkedIn Auto-Apply](docs/linkedin-auto-apply.md) | Candidate profile setup, usage, matching logic, troubleshooting |

## Project Structure

```
CloseClaw/
├── crates/
│   ├── closeclaw-core/        # Traits & types (Agent, Tool, Channel, Config)
│   ├── closeclaw-tools/       # Built-in tools (exec, file, web, browser)
│   ├── closeclaw-agent/       # ReAct runtime, LLM providers, context builder
│   ├── closeclaw-channels/    # CLI, WebChat, Telegram
│   ├── closeclaw-gateway/     # Hub, router, session store, event bus
│   └── closeclaw/             # Binary entry point
├── docs/
│   ├── browser-tool.md        # Browser tool setup & reference
│   └── linkedin-auto-apply.md # LinkedIn skill usage guide
├── scripts/
│   ├── playwright_cdp.py      # Browser automation backend (Python)
│   └── requirements.txt
├── skills/
│   └── linkedin_apply.md      # LinkedIn auto-apply skill
├── candidate_profile.toml     # Your personal info (gitignored)
├── config.default.toml
└── Cargo.toml                 # Workspace manifest
```

## License

MIT
