# How to Use CloseClaw

## Quick Start

### 1. First Run (Setup Wizard)

```bash
cargo run
```

On first launch, CloseClaw opens a setup wizard at `http://127.0.0.1:3000` where you configure:

- **LLM Provider**: Anthropic or OpenAI
- **Authentication**: API key or OAuth token (Claude subscription)
- **Model**: e.g., `claude-sonnet-4-20250514`
- **Telegram token** (optional): Get one from [@BotFather](https://t.me/BotFather)

After setup, your config is saved to `config.toml` and secrets to `~/.closeclaw/.env`.

### 2. Running the Gateway (Recommended)

```bash
cargo run -- run
```

This starts the full gateway with all channels simultaneously:

- **WebChat**: Opens at `http://127.0.0.1:3000` (configurable)
- **Telegram**: Starts long polling if configured
- **Scheduler**: Runs cron-based tasks

### 3. CLI-Only Mode

```bash
cargo run -- chat
```

A simple terminal REPL — type messages, get responses. No web UI.

### 4. Telegram-Only Mode

```bash
cargo run -- telegram
```

Runs just the Telegram bot without the web UI.

---

## Configuration

The config file is `config.toml` in your workspace (project root). See `config.default.toml` for a fully annotated reference.

### Key Sections

```toml
[gateway]
bind = "127.0.0.1"
port = 3000

[[agents]]
id = "default"
name = "CloseClaw Agent"
tools = ["exec", "read_file", "write_file", "web_fetch", "web_search", ...]
# soul_md = "SOUL.md"      # Custom persona file
# skills_dir = "/extra/skills"

[[channels]]
type = "telegram"
enabled = true
token_env = "TELOXIDE_TOKEN"

[[schedules]]
id = "morning-news"
cron = "0 0 9 * * 1-5 *"
message = "Give me a brief news summary."

[llm]
provider = "anthropic"
model = "claude-sonnet-4-20250514"
auth_mode = "api_key"          # or "oauth_token"
max_iterations = 25
```

### Authentication Modes

| Mode | How it works |
|---|---|
| `api_key` | Set `ANTHROPIC_API_KEY` or `OPENAI_API_KEY` env var (or configure via setup wizard) |
| `oauth_token` | Uses your Claude Pro/Team/Max subscription. On macOS, auto-reads from Keychain (requires `claude login` from Claude Code CLI) |

### Cron Format

7 fields: `sec min hour dom month dow year`

```
"0 0 9 * * 1-5 *"     → weekdays at 9:00 AM
"0 30 14 * * * *"      → every day at 2:30 PM
"*/10 * * * * * *"     → every 10 seconds (testing)
```

---

## WebChat

Open `http://127.0.0.1:3000` in your browser. Features:

- **Real-time streaming**: See the agent's response as it types.
- **Tool activity**: Shows which tools are being used (with spinners).
- **Cross-channel visibility**: See messages from Telegram in the WebChat sidebar.
- **Dark/light theme**: Toggle in the top-right corner.
- **Skills browser**: View all loaded skills on the Skills tab.
- **Live config editing**: Change LLM provider/model on the Config tab (some changes require restart).

The WebSocket auto-reconnects if the server restarts (2-second retry).

---

## Telegram

1. Create a bot via [@BotFather](https://t.me/BotFather) and get the token.
2. Set the token:
   - Via the setup wizard, or
   - Set `TELOXIDE_TOKEN` env var, or
   - Add it to `~/.closeclaw/.env`
3. Enable Telegram in config:
   ```toml
   [[channels]]
   type = "telegram"
   enabled = true
   token_env = "TELOXIDE_TOKEN"
   ```
4. Start with `cargo run -- run` or `cargo run -- telegram`.

### Telegram Features

- **Text messages**: Full conversation support with session persistence.
- **Photo/file uploads**: Downloads to `{workspace}/downloads/` and tells the agent the path.
- **Image responses**: If the agent mentions image file paths, they're sent as Telegram photos automatically.
- **Rich formatting**: Markdown is converted to Telegram HTML (bold, italic, code, links, headers).
- **Message splitting**: Long responses are automatically split at 4096-char boundaries.

---

## Skills

Skills are reusable prompt modules that extend the agent's capabilities. They're loaded as markdown files with YAML frontmatter.

### Using Skills

Invoke a skill with a slash command:

```
/news-briefing AI regulation in 2025
/code-review
/translate to Japanese: Hello world
```

The agent receives the skill's full instructions in its system prompt and follows them.

### Built-in Skills

| Skill | Slash command | Description |
|---|---|---|
| Code Review | `/code-review` | Review code for bugs, style, and best practices |
| Daily Planner | `/daily-planner` | Plan and organize your day |
| File Organizer | `/file-organizer` | Organize files in a directory |
| Free Movie Finder | `/free-movie-finder` | Find free legal movies online |
| LinkedIn Apply | `/linkedin-apply` | Automate LinkedIn job applications |
| News Briefing | `/news-briefing` | Search and summarize recent news |
| Research Assistant | `/research-assistant` | Deep research on any topic |
| Summarizer | `/summarizer` | Summarize text, articles, or documents |
| Translator | `/translator` | Translate text between languages |
| Weather Report | `/weather-report` | Get current weather information |
| Writing Helper | `/writing-helper` | Help with writing and editing |

### Creating Custom Skills

Create a folder in `skills/` (project-level) or `~/.closeclaw/skills/` (global):

```
skills/
  my-skill/
    SKILL.md
```

`SKILL.md` format:

```markdown
---
name: My Custom Skill
description: What this skill does in one line.
user-invocable: true
disable-model-invocation: false
metadata:
  emoji: "🔧"
  requires:
    bins: ["python3"]     # optional: gate on binary presence
    env: ["MY_API_KEY"]   # optional: gate on env var
    os: ["macos"]         # optional: gate on OS
---

# My Custom Skill

Instructions for the agent go here. This content is injected
into the system prompt when the skill is activated.

## Workflow
1. Step one...
2. Step two...
```

**Skill directories** (loaded in priority order, later overrides earlier):

1. `~/.closeclaw/skills/` — global, shared across all projects
2. `{workspace}/skills/` — project-specific
3. Agent's `skills_dir` config — additional custom directory

---

## Tools

### Built-in Tools

| Tool | Description |
|---|---|
| `exec` | Run shell commands (workspace as working directory, 30s timeout) |
| `read_file` | Read file contents (absolute or workspace-relative path) |
| `write_file` | Write/create files (creates parent directories) |
| `web_fetch` | Fetch a URL and convert HTML to plain text |
| `web_search` | Search the web via Brave Search (no API key needed) |
| `list_files` | List directory contents with type and size |
| `create_file` | Create or overwrite a file |
| `delete_file` | Delete a file or directory |
| `search_files` | Search by filename pattern and/or content |
| `browser` | Full browser automation via `browser-use` |
| `browser_cdp` | Chrome DevTools Protocol browser control |
| `self_manage` | Server self-management (restart) |
| `add_schedule` | Add a cron schedule at runtime |
| `remove_schedule` | Remove a dynamic schedule |
| `list_schedules` | List all active schedules |

### Self-Management

Tell the agent "restart yourself" and it will:

1. Notify you that a restart is coming.
2. Restart the server process in-place (same binary, same args).
3. Notify you when the restart is complete.

---

## Schedules

### Config-based Schedules

Add to `config.toml`:

```toml
[[schedules]]
id = "daily-standup"
cron = "0 0 9 * * 1-5 *"
message = "What are the top 3 things I should focus on today?"
enabled = true
```

### Dynamic Schedules (via Chat)

Ask the agent in any channel:

> "Schedule a daily weather report at 8 AM on weekdays"

The agent will use the `add_schedule` tool. Dynamic schedules persist in `~/.closeclaw/schedules.json` and survive restarts.

If you created a schedule from Telegram, responses are automatically delivered back to your Telegram chat.

### Managing Schedules

- "List my schedules" → agent calls `list_schedules`
- "Remove the morning-news schedule" → agent calls `remove_schedule`

---

## Persona (SOUL.md)

Place a `SOUL.md` file in your workspace root to customize the agent's personality and behavior. This is injected into the system prompt on every request.

Example:

```markdown
# My Assistant

You are a senior software engineer assistant.
Always respond concisely. Prefer code examples over long explanations.
When unsure, say so rather than guessing.
```

Other optional workspace files that are auto-loaded into the system prompt:

| File | Purpose |
|---|---|
| `SOUL.md` | Agent persona and behavior rules |
| `AGENTS.md` | Multi-agent coordination instructions |
| `TOOLS.md` | Custom tool usage guidelines |
| `IDENTITY.md` | Agent identity information |
| `USER.md` | User profile/context |

---

## File Paths

| Path | Purpose |
|---|---|
| `config.toml` | Main configuration |
| `SOUL.md` | Agent persona |
| `skills/` | Project-level skills |
| `~/.closeclaw/.env` | Stored API keys and tokens |
| `~/.closeclaw/skills/` | Global skills |
| `~/.closeclaw/sessions/` | Persisted chat histories (JSONL) |
| `~/.closeclaw/schedules.json` | Dynamic schedule persistence |
| `downloads/` | Telegram file downloads |
