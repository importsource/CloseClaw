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
| `browser` | Browser automation via Chrome/Edge CDP + Playwright (see [setup](#browser-tool-setup)) |

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

**OAuth mode** (compatible with Claude Code):

```toml
[llm]
auth_mode = "oauth_token"
token_env = "CLAUDE_CODE_TOKEN"  # or omit to auto-read from macOS Keychain
```

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

## Browser Tool Setup

The `browser` tool lets the agent control a real Chrome or Edge browser. It uses Chrome DevTools Protocol (CDP) under the hood: the Rust tool launches the browser and delegates actions to a Python Playwright script.

### Prerequisites

**1. Python 3.8+**

```bash
python3 --version   # must be 3.8 or above
```

**2. Install Playwright**

```bash
pip3 install playwright
python3 -m playwright install chromium
```

The `chromium` install is needed even though we connect to your system Chrome/Edge via CDP — Playwright requires the browser binaries to be present for the library to load.

**3. A supported browser**

The tool auto-detects (in order of preference):

| Priority | Browser | macOS Path |
|----------|---------|------------|
| 1 | Microsoft Edge | `/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge` |
| 2 | Google Chrome | `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome` |

No manual configuration needed — just have one of these installed.

**4. Create screenshots directory**

```bash
mkdir -p screenshots
```

### How it works

```
Agent (LLM)
  │
  ▼
BrowserTool (Rust)
  │  ┌─ "launch" action → spawns Chrome/Edge with --remote-debugging-port=9222
  │  └─ all other actions → spawns python3 scripts/playwright_cdp.py
  ▼
Playwright (Python) ──CDP──▶ Chrome/Edge
```

- The browser launches with a dedicated profile dir (`.browser-profile/`) so your personal browser is unaffected.
- Each action invocation is stateless — the Python script connects via CDP, performs the action, and exits.
- The browser stays running between actions, preserving login sessions and cookies.

### Supported browser actions

| Action | Description |
|--------|-------------|
| `launch` | Start the browser (or confirm it's running) |
| `navigate` | Go to a URL |
| `click` | Click an element by CSS selector |
| `type` | Type text into an input field |
| `press_key` | Press a keyboard key (Enter, Tab, etc.) |
| `screenshot` | Take a screenshot (saved to `screenshots/`) |
| `get_text` | Get visible text of an element |
| `get_html` | Get HTML of an element |
| `evaluate` | Run arbitrary JavaScript in the page |
| `select_option` | Select a dropdown option |
| `check` | Check a checkbox |
| `upload_file` | Upload a file to a file input |
| `scroll` | Scroll the page |
| `wait_for_selector` | Wait for an element to appear |
| `query_selector_all` | Find all elements matching a selector |
| `new_tab` / `close_tab` | Tab management |
| `list_tabs` | List all open tabs |

### Verify installation

```bash
# 1. Make sure Playwright is installed
python3 -c "from playwright.sync_api import sync_playwright; print('OK')"

# 2. Build CloseClaw
cargo build --release

# 3. Start CloseClaw
./target/release/closeclaw run --config config.toml

# 4. Tell the agent (via Telegram, WebChat, or CLI):
#    "Launch the browser"
#    → Chrome/Edge should open
```

## LinkedIn Auto-Apply Skill

A built-in skill (`skills/linkedin_apply.md`) that teaches the agent to search LinkedIn jobs, match them against your resume, and apply via Easy Apply — fully autonomously.

### Setup

**1. Complete the browser tool setup above.**

**2. Create your candidate profile**

Copy the template and fill in your details:

```bash
cp candidate_profile.toml.example candidate_profile.toml
# Or create it from scratch — see format below
```

```toml
# candidate_profile.toml

[personal]
first_name = "Jane"
last_name = "Doe"
full_name = "Jane Doe"
email = "jane@example.com"
phone = "5551234567"
phone_country = "United States (+1)"
city = "San Francisco, CA"
state = "California"
zip = "94105"
website = "https://janedoe.dev"
linkedin = "https://www.linkedin.com/in/janedoe/"

[work]
current_company = "Acme Corp"
current_title = "Senior Software Engineer"
years_of_experience = 8
work_authorization = "Yes"
sponsorship_required = "No"
willing_to_relocate = "Yes"
remote_ok = "Yes"
start_date = "Immediately"
salary_expectation = 180000

[resume]
# Path relative to workspace root
file = "Resume-Jane.pdf"

[cover_letter]
default = "Your brief cover letter / elevator pitch here."

[skills]
core = ["Python", "Go", "Kubernetes", "AWS", "PostgreSQL"]
ai = ["LLM", "RAG", "Fine-tuning"]
languages = ["Python", "Go", "SQL"]

[target_roles]
titles = ["Senior Software Engineer", "Staff Engineer", "Backend Engineer"]
keywords_match = ["Python", "Go", "distributed systems", "microservices", "cloud"]
keywords_skip = ["frontend-only", "iOS", "Android", "security clearance"]
min_years_acceptable = 5
location = "San Francisco"

[demographics]
gender = "Prefer not to say"
race = "Decline to self-identify"
veteran = "Prefer not to say"
disability = "Prefer not to say"

[defaults]
how_heard = "LinkedIn"
```

**3. Place your resume PDF in the workspace root**

```bash
cp /path/to/your/Resume.pdf ./Resume-Jane.pdf
```

Make sure the filename matches `[resume].file` in your profile.

**4. Increase max iterations**

LinkedIn apply needs many tool calls. In your `config.toml`:

```toml
[llm]
max_iterations = 100   # default 25 is too low for job applications
```

### Usage

**First run — log in to LinkedIn:**

1. Start CloseClaw: `./target/release/closeclaw run`
2. Tell the agent: **"Launch the browser"**
3. Edge/Chrome opens with a clean profile. **Log in to LinkedIn manually** in the browser window.
4. Your session is saved in `.browser-profile/` — you only need to log in once.

**Apply for jobs:**

Tell the agent (via Telegram, WebChat, or CLI):

> "Search for software engineer jobs in San Francisco and apply to matching ones"

The agent will:

1. Read your `candidate_profile.toml`
2. Navigate to LinkedIn job search
3. Extract all job listings
4. For each job:
   - Read the job description
   - Match against your `keywords_match` / `keywords_skip`
   - If good match + Easy Apply available → fill the form and submit
   - If poor match or external site → skip
5. Send you a single summary at the end:
   - Applied: list of jobs
   - Skipped: list with reasons
   - Total: X applied, Y skipped out of Z found

Screenshots are taken after each submission and sent automatically if you're using Telegram.

### Important notes

- The agent runs **fully autonomously** — it will not ask for confirmation between jobs.
- The **only** time it stops is if it hits a CAPTCHA or security check.
- `candidate_profile.toml` and `Resume-*.pdf` are gitignored — your personal data stays local.
- The `.browser-profile/` directory is also gitignored.
- If the agent runs out of iterations, increase `max_iterations` in `config.toml`.

### Troubleshooting

| Problem | Fix |
|---------|-----|
| Browser won't launch | Check that Edge or Chrome is installed. Run `python3 -c "from playwright.sync_api import sync_playwright; print('OK')"` |
| "CDP connection refused" | The browser may have crashed. Tell the agent "Launch the browser" again. |
| Agent runs out of iterations | Set `max_iterations = 100` (or higher) in `config.toml` |
| LinkedIn shows "Sign in" | Your session expired. Tell the agent to launch the browser, log in manually, then retry. |
| Form fields not filling | Check that `candidate_profile.toml` has the relevant fields. The agent uses best judgment for unknown fields. |
| Screenshots not appearing in Telegram | Make sure the `screenshots/` directory exists |

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
