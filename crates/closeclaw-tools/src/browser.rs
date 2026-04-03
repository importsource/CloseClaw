use async_trait::async_trait;
use closeclaw_core::error::Result;
use closeclaw_core::tool::{Tool, ToolDefinition, ToolResult};
use serde_json::{json, Value};
use std::path::PathBuf;
use tracing::{debug, warn};

/// Browser tool backed by [Browser Use CLI 2.0](https://docs.browser-use.com/open-source/browser-use-cli).
///
/// Every action is delegated to the `browser-use` CLI which manages a persistent
/// browser daemon with ~50ms latency, session management, and automatic element
/// indexing.
pub struct BrowserTool {
    workspace: PathBuf,
    session: String,
}

impl BrowserTool {
    pub fn new(workspace: PathBuf) -> Self {
        Self {
            workspace,
            session: "closeclaw".to_string(),
        }
    }

    /// Run `browser-use --session <session> --json <args...>` and return stdout.
    async fn run_browser_use(
        &self,
        args: &[&str],
        global_flags: &[&str],
    ) -> std::result::Result<String, String> {
        let mut cmd = tokio::process::Command::new("browser-use");
        cmd.arg("--session").arg(&self.session).arg("--json");

        for flag in global_flags {
            cmd.arg(flag);
        }

        cmd.args(args);
        cmd.current_dir(&self.workspace);

        debug!(args = ?args, session = %self.session, "Running browser-use");

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            cmd.output(),
        )
        .await
        .map_err(|_| "browser-use command timed out after 60s".to_string())?
        .map_err(|e| {
            format!(
                "Failed to run browser-use. Is it installed? (pip install browser-use)\nError: {e}"
            )
        })?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        if !output.status.success() {
            let msg = if !stderr.trim().is_empty() {
                stderr.trim().to_string()
            } else if !stdout.trim().is_empty() {
                stdout.trim().to_string()
            } else {
                format!("browser-use exited with status {}", output.status)
            };
            return Err(msg);
        }

        if stdout.trim().is_empty() {
            // Some commands (e.g. close) may produce no output on success
            return Ok("OK".to_string());
        }

        Ok(stdout)
    }

    /// Extract a string param from the JSON value, with optional fallback key.
    fn str_param<'a>(params: &'a Value, key: &str) -> Option<&'a str> {
        params.get(key).and_then(|v| v.as_str())
    }

    /// Extract a numeric param as a string.
    fn num_param_str(params: &Value, key: &str) -> Option<String> {
        params.get(key).and_then(|v| {
            if let Some(n) = v.as_i64() {
                Some(n.to_string())
            } else if let Some(n) = v.as_f64() {
                Some(n.to_string())
            } else {
                v.as_str().map(|s| s.to_string())
            }
        })
    }

    /// Collect global flags (--headed, --profile) from params.
    fn global_flags(params: &Value) -> Vec<String> {
        let mut flags = Vec::new();
        if params.get("headed").and_then(|v| v.as_bool()).unwrap_or(false) {
            flags.push("--headed".to_string());
        }
        if let Some(profile) = Self::str_param(params, "profile") {
            flags.push("--profile".to_string());
            flags.push(profile.to_string());
        }
        flags
    }
}

#[async_trait]
impl Tool for BrowserTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "browser".to_string(),
            description: "Control a browser via Browser Use CLI. Actions: \
                open (navigate to URL), state (get page URL/title/elements), \
                click (click element by index or coordinates), type (send keystrokes), \
                input (fill element by index), keys (press key combos like Enter/Escape), \
                screenshot (capture page), scroll (up/down/left/right), back (go back), \
                get_text (extract text), get_html (get page HTML), get_value (get input value), \
                select (choose dropdown option), upload (file upload), hover (hover element), \
                eval (run JavaScript), wait (wait for selector/text), switch_tab, close_tab, \
                sessions (list active sessions), close (end session). \
                Screenshots are saved as PNG files -- include the file path in your response for the user to see."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "description": "The browser action to perform.",
                        "enum": [
                            "open", "state", "click", "type", "input", "keys",
                            "screenshot", "scroll", "back",
                            "get_text", "get_html", "get_value",
                            "select", "upload", "hover",
                            "eval", "wait", "switch_tab", "close_tab",
                            "sessions", "close"
                        ]
                    },
                    "params": {
                        "type": "object",
                        "description": "Action-specific parameters:\n\
                            - open: {url} — navigate to URL\n\
                            - state: {} — returns current URL, title, and clickable elements with indices\n\
                            - click: {index} or {x, y} — click element by index or coordinates\n\
                            - type: {text} — type text (sends keystrokes to focused element)\n\
                            - input: {index, text} — clear and fill element by index\n\
                            - keys: {key} — press key combo, e.g. \"Enter\", \"Control+a\", \"Escape\"\n\
                            - screenshot: {path?} — save screenshot (default: screenshots/<timestamp>.png)\n\
                            - scroll: {direction?, amount?} — direction: up/down/left/right (default: down), amount in pixels\n\
                            - back: {} — navigate back\n\
                            - get_text: {index?} — get text content of element (or full page)\n\
                            - get_html: {} — get page HTML\n\
                            - get_value: {index} — get value of input element\n\
                            - select: {index, value} — select dropdown option by value\n\
                            - upload: {index, path} — upload file to file input\n\
                            - hover: {index} — hover over element\n\
                            - eval: {expression} — run JavaScript and return result\n\
                            - wait: {selector?, text?, timeout?} — wait for CSS selector or text to appear\n\
                            - switch_tab: {tab} — switch to tab by index\n\
                            - close_tab: {} — close current tab\n\
                            - sessions: {} — list active browser sessions\n\
                            - close: {} — close the browser session\n\n\
                            Global flags (can be added to any action):\n\
                            - headed: true — show the browser window (default: headless)\n\
                            - profile: \"name\" — use a Chrome profile for persistent login state"
                    }
                },
                "required": ["action"]
            }),
        }
    }

    async fn execute(&self, input: Value) -> Result<ToolResult> {
        let action = input["action"].as_str().ok_or_else(|| {
            closeclaw_core::error::CloseClawError::Tool("missing 'action' field".into())
        })?;

        // Build params: merge explicit "params" with any top-level keys
        // (handles LLMs that send flat params alongside "action").
        let mut params = input.get("params").cloned().unwrap_or_else(|| json!({}));
        if let (Some(params_obj), Some(input_obj)) = (params.as_object_mut(), input.as_object()) {
            for (k, v) in input_obj {
                if k != "action" && k != "params" {
                    params_obj.entry(k.clone()).or_insert_with(|| v.clone());
                }
            }
        }

        let global = Self::global_flags(&params);
        let global_refs: Vec<&str> = global.iter().map(|s| s.as_str()).collect();

        let result = match action {
            "open" => {
                let url = Self::str_param(&params, "url")
                    .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool(
                        "open requires 'url' parameter".into(),
                    ))?;
                self.run_browser_use(&["open", url], &global_refs).await
            }

            "state" => {
                self.run_browser_use(&["state"], &global_refs).await
            }

            "click" => {
                if let Some(index) = Self::num_param_str(&params, "index") {
                    self.run_browser_use(&["click", &index], &global_refs).await
                } else if let (Some(x), Some(y)) = (
                    Self::num_param_str(&params, "x"),
                    Self::num_param_str(&params, "y"),
                ) {
                    self.run_browser_use(&["click", &x, &y], &global_refs).await
                } else {
                    Err("click requires 'index' or ('x','y') parameters".to_string())
                }
            }

            "type" => {
                let text = Self::str_param(&params, "text")
                    .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool(
                        "type requires 'text' parameter".into(),
                    ))?;
                self.run_browser_use(&["type", text], &global_refs).await
            }

            "input" => {
                let index = Self::num_param_str(&params, "index")
                    .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool(
                        "input requires 'index' parameter".into(),
                    ))?;
                let text = Self::str_param(&params, "text")
                    .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool(
                        "input requires 'text' parameter".into(),
                    ))?;
                self.run_browser_use(&["input", &index, text], &global_refs).await
            }

            "keys" => {
                let key = Self::str_param(&params, "key")
                    .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool(
                        "keys requires 'key' parameter".into(),
                    ))?;
                self.run_browser_use(&["keys", key], &global_refs).await
            }

            "screenshot" => {
                let screenshots_dir = self.workspace.join("screenshots");
                let _ = std::fs::create_dir_all(&screenshots_dir);

                let ts = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_secs();
                let default_path = screenshots_dir
                    .join(format!("{ts}.png"))
                    .to_string_lossy()
                    .to_string();

                let path = Self::str_param(&params, "path")
                    .map(|s| s.to_string())
                    .unwrap_or(default_path);

                self.run_browser_use(&["screenshot", &path], &global_refs).await
            }

            "scroll" => {
                let direction = Self::str_param(&params, "direction").unwrap_or("down");
                let mut args = vec!["scroll", direction];
                let amount;
                if let Some(a) = Self::num_param_str(&params, "amount") {
                    amount = a;
                    args.push("--amount");
                    args.push(&amount);
                }
                self.run_browser_use(&args, &global_refs).await
            }

            "back" => {
                self.run_browser_use(&["back"], &global_refs).await
            }

            "get_text" => {
                if let Some(index) = Self::num_param_str(&params, "index") {
                    self.run_browser_use(&["get", "text", &index], &global_refs).await
                } else {
                    self.run_browser_use(&["get", "text"], &global_refs).await
                }
            }

            "get_html" => {
                self.run_browser_use(&["get", "html"], &global_refs).await
            }

            "get_value" => {
                let index = Self::num_param_str(&params, "index")
                    .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool(
                        "get_value requires 'index' parameter".into(),
                    ))?;
                self.run_browser_use(&["get", "value", &index], &global_refs).await
            }

            "select" => {
                let index = Self::num_param_str(&params, "index")
                    .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool(
                        "select requires 'index' parameter".into(),
                    ))?;
                let value = Self::str_param(&params, "value")
                    .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool(
                        "select requires 'value' parameter".into(),
                    ))?;
                self.run_browser_use(&["select", &index, value], &global_refs).await
            }

            "upload" => {
                let index = Self::num_param_str(&params, "index")
                    .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool(
                        "upload requires 'index' parameter".into(),
                    ))?;
                let path = Self::str_param(&params, "path")
                    .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool(
                        "upload requires 'path' parameter".into(),
                    ))?;
                self.run_browser_use(&["upload", &index, path], &global_refs).await
            }

            "hover" => {
                let index = Self::num_param_str(&params, "index")
                    .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool(
                        "hover requires 'index' parameter".into(),
                    ))?;
                self.run_browser_use(&["hover", &index], &global_refs).await
            }

            "eval" => {
                let expression = Self::str_param(&params, "expression")
                    .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool(
                        "eval requires 'expression' parameter".into(),
                    ))?;
                self.run_browser_use(&["eval", expression], &global_refs).await
            }

            "wait" => {
                if let Some(selector) = Self::str_param(&params, "selector") {
                    let mut args = vec!["wait", "selector", selector];
                    let timeout;
                    if let Some(t) = Self::num_param_str(&params, "timeout") {
                        timeout = t;
                        args.push("--timeout");
                        args.push(&timeout);
                    }
                    self.run_browser_use(&args, &global_refs).await
                } else if let Some(text) = Self::str_param(&params, "text") {
                    let mut args = vec!["wait", "text", text];
                    let timeout;
                    if let Some(t) = Self::num_param_str(&params, "timeout") {
                        timeout = t;
                        args.push("--timeout");
                        args.push(&timeout);
                    }
                    self.run_browser_use(&args, &global_refs).await
                } else {
                    Err("wait requires 'selector' or 'text' parameter".to_string())
                }
            }

            "switch_tab" => {
                let tab = Self::num_param_str(&params, "tab")
                    .ok_or_else(|| closeclaw_core::error::CloseClawError::Tool(
                        "switch_tab requires 'tab' parameter".into(),
                    ))?;
                self.run_browser_use(&["switch", &tab], &global_refs).await
            }

            "close_tab" => {
                self.run_browser_use(&["close-tab"], &global_refs).await
            }

            "sessions" => {
                self.run_browser_use(&["sessions"], &global_refs).await
            }

            "close" => {
                self.run_browser_use(&["close"], &global_refs).await
            }

            other => {
                warn!(action = other, "Unknown browser action");
                Err(format!("Unknown browser action: '{other}'. Use 'state' to see available actions."))
            }
        };

        match result {
            Ok(output) => Ok(ToolResult::success(output)),
            Err(e) => {
                warn!(action, error = %e, "Browser action failed");
                Ok(ToolResult::error(e))
            }
        }
    }
}
