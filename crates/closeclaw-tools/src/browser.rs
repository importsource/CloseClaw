use async_trait::async_trait;
use closeclaw_core::error::Result;
use closeclaw_core::tool::{Tool, ToolDefinition, ToolResult};
use serde_json::{json, Value};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tracing::{info, warn};

pub struct BrowserTool {
    workspace: PathBuf,
    launched: AtomicBool,
    cdp_url: Mutex<String>,
    pid: Mutex<Option<u32>>,
}

impl BrowserTool {
    pub fn new(workspace: PathBuf) -> Self {
        Self {
            workspace,
            launched: AtomicBool::new(false),
            cdp_url: Mutex::new(String::new()),
            pid: Mutex::new(None),
        }
    }

    /// Find Chrome or Edge binary on macOS.
    fn find_browser_binary() -> Option<PathBuf> {
        let candidates = [
            "/Applications/Microsoft Edge.app/Contents/MacOS/Microsoft Edge",
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "/Applications/Chromium.app/Contents/MacOS/Chromium",
        ];
        for path in &candidates {
            let p = PathBuf::from(path);
            if p.exists() {
                return Some(p);
            }
        }
        None
    }

    /// Check if Chrome process is still alive.
    fn is_chrome_alive(&self) -> bool {
        let pid = *self.pid.lock().unwrap();
        match pid {
            Some(p) => {
                // Use kill -0 via shell to check if process exists
                std::process::Command::new("kill")
                    .args(["-0", &p.to_string()])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .map(|s| s.success())
                    .unwrap_or(false)
            }
            None => false,
        }
    }

    /// Reset launch state (when Chrome has died).
    fn reset_launch_state(&self) {
        self.launched.store(false, Ordering::SeqCst);
        *self.pid.lock().unwrap() = None;
        *self.cdp_url.lock().unwrap() = String::new();
        warn!("Browser process is dead, resetting launch state");
    }

    /// Launch Chrome with remote debugging enabled.
    fn launch_browser(
        &self,
        port: u16,
        headless: bool,
        url: Option<&str>,
    ) -> std::result::Result<(String, u32), String> {
        let binary = Self::find_browser_binary()
            .ok_or_else(|| "No Chrome/Edge/Chromium found in /Applications".to_string())?;

        let profile_dir = self.workspace.join(".browser-profile");
        std::fs::create_dir_all(&profile_dir)
            .map_err(|e| format!("Failed to create profile dir: {e}"))?;

        // Also create screenshots dir
        let screenshots_dir = self.workspace.join("screenshots");
        let _ = std::fs::create_dir_all(screenshots_dir);

        let mut args = vec![
            format!("--remote-debugging-port={port}"),
            format!("--user-data-dir={}", profile_dir.display()),
            "--no-first-run".to_string(),
            "--no-default-browser-check".to_string(),
        ];

        if headless {
            args.push("--headless=new".to_string());
        }

        if let Some(u) = url {
            args.push(u.to_string());
        }

        info!(binary = %binary.display(), port, "Launching browser");

        let child = std::process::Command::new(&binary)
            .args(&args)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .map_err(|e| format!("Failed to spawn browser: {e}"))?;

        let pid = child.id();
        let cdp = format!("http://127.0.0.1:{port}");

        *self.pid.lock().unwrap() = Some(pid);
        self.cdp_url.lock().unwrap().clone_from(&cdp);
        self.launched.store(true, Ordering::SeqCst);

        info!(pid, cdp_url = %cdp, "Browser launched");
        Ok((cdp, pid))
    }

    /// Wait for CDP endpoint to become available (retry with backoff).
    async fn wait_for_cdp(&self, port: u16) -> std::result::Result<(), String> {
        let url = format!("http://127.0.0.1:{port}/json/version");
        for attempt in 0..10 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;

            let ok = tokio::process::Command::new("curl")
                .args(["-s", "-o", "/dev/null", "-w", "%{http_code}", &url])
                .output()
                .await
                .map(|o| String::from_utf8_lossy(&o.stdout).trim() == "200")
                .unwrap_or(false);

            if ok {
                info!(attempt, "CDP endpoint ready");
                return Ok(());
            }

            // Check if Chrome is still alive
            if !self.is_chrome_alive() {
                self.reset_launch_state();
                return Err("Browser process died during startup".to_string());
            }
        }
        Err("CDP endpoint did not become ready within 5 seconds".to_string())
    }

    /// Delegate an action to the Python Playwright script.
    async fn run_playwright_action(
        &self,
        action_json: &str,
    ) -> std::result::Result<Value, String> {
        let cdp = self.cdp_url.lock().unwrap().clone();
        if cdp.is_empty() {
            return Err("Browser not launched. Call with action='launch' first.".to_string());
        }

        let script = self.workspace.join("scripts/playwright_cdp.py");
        if !script.exists() {
            return Err(format!(
                "Playwright script not found at {}",
                script.display()
            ));
        }

        let output = tokio::time::timeout(
            std::time::Duration::from_secs(60),
            tokio::process::Command::new("python3")
                .arg(&script)
                .arg(&cdp)
                .arg(action_json)
                .current_dir(&self.workspace)
                .output(),
        )
        .await
        .map_err(|_| "Playwright action timed out after 60s".to_string())?
        .map_err(|e| format!("Failed to run playwright script: {e}"))?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let stderr = String::from_utf8_lossy(&output.stderr);

        if stdout.trim().is_empty() {
            return Err(format!(
                "Playwright script returned no output. stderr: {stderr}"
            ));
        }

        let parsed: Value = serde_json::from_str(stdout.trim())
            .map_err(|e| format!("Failed to parse Playwright output: {e}\nstdout: {stdout}"))?;

        if let Some(err) = parsed.get("error").and_then(|v| v.as_str()) {
            return Err(err.to_string());
        }

        Ok(parsed)
    }
}

#[async_trait]
impl Tool for BrowserTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "browser".to_string(),
            description: "Control a Chrome/Edge browser via CDP. Use action='launch' to start, \
                then navigate, click, type, screenshot, etc. Screenshots are saved as PNG files \
                -- include the file path in your response for the user to see."
                .to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "action": {
                        "type": "string",
                        "description": "The browser action to perform.",
                        "enum": [
                            "launch", "list_tabs", "navigate", "click", "type", "press_key",
                            "screenshot", "get_text", "get_html", "get_attribute",
                            "query_selector_all", "wait_for_selector", "wait_for_navigation",
                            "evaluate", "select_option", "check", "upload_file",
                            "scroll", "new_tab", "close_tab"
                        ]
                    },
                    "params": {
                        "type": "object",
                        "description": "Action-specific parameters. For launch: {port?, headless?, url?}. For navigate: {url}. For click: {selector}. For type: {selector, text, clear?, delay?}. For press_key: {key}. For screenshot: {path, full_page?, selector?}. For get_text: {selector?}. For get_html: {selector?, outer?}. For get_attribute: {selector, attribute}. For query_selector_all: {selector, max_results?}. For wait_for_selector: {selector, timeout?, state?}. For evaluate: {expression}. For select_option: {selector, value?, label?}. For check: {selector, checked?}. For upload_file: {selector, file_path}. For scroll: {direction?, amount?}. For new_tab: {url?}. For close_tab: {}. Common: tab_index (default 0)."
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

        // Build params: start from explicit "params" object, then merge any
        // top-level keys the LLM may have placed outside of "params".
        // This handles LLMs that send {"action":"navigate","url":"..."} instead
        // of {"action":"navigate","params":{"url":"..."}}.
        let mut params = input.get("params").cloned().unwrap_or_else(|| json!({}));
        if let (Some(params_obj), Some(input_obj)) = (params.as_object_mut(), input.as_object()) {
            for (k, v) in input_obj {
                if k != "action" && k != "params" {
                    params_obj.entry(k.clone()).or_insert_with(|| v.clone());
                }
            }
        }

        match action {
            "launch" => {
                // If we think Chrome is running, check if it's actually alive
                if self.launched.load(Ordering::SeqCst) {
                    if self.is_chrome_alive() {
                        let cdp = self.cdp_url.lock().unwrap().clone();
                        let ws = self.workspace.display();
                        return Ok(ToolResult::success(format!(
                            "Browser already running. CDP endpoint: {cdp}\n\
                             Workspace: {ws}\n\
                             Screenshots dir: {ws}/screenshots/"
                        )));
                    }
                    // Chrome died — reset and re-launch
                    self.reset_launch_state();
                }

                let port =
                    params.get("port").and_then(|v| v.as_u64()).unwrap_or(9222) as u16;
                let headless = params
                    .get("headless")
                    .and_then(|v| v.as_bool())
                    .unwrap_or(false);
                let url = params.get("url").and_then(|v| v.as_str());

                match self.launch_browser(port, headless, url) {
                    Ok((cdp, pid)) => {
                        // Wait for CDP to become available (up to 5s with retries)
                        match self.wait_for_cdp(port).await {
                            Ok(()) => {
                                let ws = self.workspace.display();
                                Ok(ToolResult::success(format!(
                                    "Browser launched and CDP ready.\n\
                                     CDP endpoint: {cdp}\n\
                                     PID: {pid}\n\
                                     Workspace: {ws}\n\
                                     Screenshots dir: {ws}/screenshots/\n\n\
                                     IMPORTANT: When saving screenshots, use the absolute path \
                                     {ws}/screenshots/<name>.png\n\n\
                                     If this is the first run, navigate to https://www.linkedin.com \
                                     and log in manually. The session persists across restarts."
                                )))
                            }
                            Err(e) => Ok(ToolResult::error(format!(
                                "Browser spawned but CDP not ready: {e}"
                            ))),
                        }
                    }
                    Err(e) => Ok(ToolResult::error(e)),
                }
            }
            _ => {
                // Check if Chrome is still alive before delegating
                if !self.launched.load(Ordering::SeqCst) || !self.is_chrome_alive() {
                    if self.launched.load(Ordering::SeqCst) {
                        self.reset_launch_state();
                    }
                    return Ok(ToolResult::error(
                        "Browser not running. Call with action='launch' first.",
                    ));
                }

                let action_json = json!({
                    "action": action,
                    "params": params,
                });

                match self.run_playwright_action(&action_json.to_string()).await {
                    Ok(result) => Ok(ToolResult::success(
                        serde_json::to_string_pretty(&result).unwrap_or_default(),
                    )),
                    Err(e) => {
                        warn!(action, error = %e, "Browser action failed");
                        Ok(ToolResult::error(e))
                    }
                }
            }
        }
    }
}
