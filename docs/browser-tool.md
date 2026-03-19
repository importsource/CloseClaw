# Browser Tool

The `browser` tool lets the agent control a real Chrome or Edge browser. It uses Chrome DevTools Protocol (CDP) under the hood: the Rust tool launches the browser and delegates actions to a Python Playwright script.

## Prerequisites

### 1. Python 3.8+

```bash
python3 --version   # must be 3.8 or above
```

### 2. Install Playwright

```bash
pip3 install playwright
python3 -m playwright install chromium
```

The `chromium` install is needed even though we connect to your system Chrome/Edge via CDP — Playwright requires the browser binaries to be present for the library to load.

### 3. A supported browser

The tool auto-detects (in order of preference):

| Priority | Browser | macOS Path |
|----------|---------|------------|
| 1 | Microsoft Edge | `/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge` |
| 2 | Google Chrome | `/Applications/Google Chrome.app/Contents/MacOS/Google Chrome` |

No manual configuration needed — just have one of these installed.

### 4. Create screenshots directory

```bash
mkdir -p screenshots
```

## How it works

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

## Supported actions

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

## Verify installation

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

## Troubleshooting

| Problem | Fix |
|---------|-----|
| Browser won't launch | Check that Edge or Chrome is installed. Run `python3 -c "from playwright.sync_api import sync_playwright; print('OK')"` |
| "CDP connection refused" | The browser may have crashed. Tell the agent "Launch the browser" again. |
| Screenshots not appearing in Telegram | Make sure the `screenshots/` directory exists |
