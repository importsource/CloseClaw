use async_trait::async_trait;
use closeclaw_core::error::Result;
use closeclaw_core::tool::{Tool, ToolDefinition, ToolResult};
use serde_json::{json, Value};

/// Web search tool using Brave Search HTML.
pub struct WebSearchTool {
    client: reqwest::Client,
}

impl WebSearchTool {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::builder()
                .timeout(std::time::Duration::from_secs(15))
                .build()
                .expect("failed to build HTTP client"),
        }
    }
}

impl Default for WebSearchTool {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl Tool for WebSearchTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "web_search".to_string(),
            description: "Search the web for information. Returns search results with titles, URLs, and descriptions. Use this to find news, information, or discover URLs to fetch with web_fetch.".to_string(),
            parameters: json!({
                "type": "object",
                "properties": {
                    "query": {
                        "type": "string",
                        "description": "The search query"
                    }
                },
                "required": ["query"]
            }),
        }
    }

    async fn execute(&self, input: Value) -> Result<ToolResult> {
        let query = input["query"]
            .as_str()
            .ok_or_else(|| {
                closeclaw_core::error::CloseClawError::Tool("missing 'query' field".into())
            })?;

        let url = format!(
            "https://search.brave.com/search?q={}&source=web",
            urlencoded(query)
        );

        let response = match self
            .client
            .get(&url)
            .header(
                "User-Agent",
                "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
            )
            .header("Accept", "text/html")
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => return Ok(ToolResult::error(format!("Search request failed: {e}"))),
        };

        if !response.status().is_success() {
            return Ok(ToolResult::error(format!("HTTP {}", response.status())));
        }

        let body = match response.text().await {
            Ok(b) => b,
            Err(e) => return Ok(ToolResult::error(format!("Failed to read response: {e}"))),
        };

        let results = parse_brave_results(&body);

        if results.is_empty() {
            Ok(ToolResult::success(format!(
                "No results found for: \"{query}\""
            )))
        } else {
            Ok(ToolResult::success(results))
        }
    }
}

/// Simple percent-encoding for query strings.
fn urlencoded(s: &str) -> String {
    let mut result = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                result.push(b as char);
            }
            b' ' => result.push('+'),
            _ => {
                result.push_str(&format!("%{:02X}", b));
            }
        }
    }
    result
}

/// Extract search results from Brave Search HTML.
/// Each result is a `<div data-pos="N" data-type="web">` block containing
/// a heading link, title, and description.
fn parse_brave_results(html: &str) -> String {
    let mut results = Vec::new();

    // Find each web result by data-pos + data-type="web"
    let mut search_from = 0;
    while let Some(pos_idx) = html[search_from..].find("data-type=\"web\"") {
        let abs_pos = search_from + pos_idx;

        // Find the data-pos value nearby (within the same tag)
        let tag_start = html[..abs_pos].rfind('<').unwrap_or(abs_pos);
        let tag_region = &html[tag_start..abs_pos + 20];
        let rank = extract_between(tag_region, "data-pos=\"", "\"")
            .and_then(|s| s.parse::<usize>().ok());

        // Determine the block boundary (next data-pos or a generous limit)
        let block_end = html[abs_pos + 15..]
            .find("data-pos=\"")
            .map(|p| abs_pos + 15 + p)
            .unwrap_or_else(|| (abs_pos + 5000).min(html.len()));
        let block = &html[abs_pos..block_end];

        // Extract the first external URL
        let url = find_first_external_url(block);

        // Extract title from <div class="...title...">...</div>
        let title = extract_class_content(block, "title");

        // Extract description
        let desc = extract_class_content(block, "description");

        if let Some(u) = url {
            let t = title.unwrap_or_default();
            let num = rank.map(|r| r + 1).unwrap_or(results.len() + 1);
            let clean_title = strip_html_tags(&t);
            let clean_desc = strip_html_tags(&desc.unwrap_or_default());
            if !clean_title.is_empty() {
                let entry = if clean_desc.is_empty() {
                    format!("{}. {}\n   {}", num, clean_title, u)
                } else {
                    format!("{}. {}\n   {}\n   {}", num, clean_title, u, clean_desc)
                };
                results.push(entry);
            }
        }

        search_from = abs_pos + 15;

        if results.len() >= 10 {
            break;
        }
    }

    results.join("\n\n")
}

/// Find the first https:// URL in a block that isn't from brave.com.
fn find_first_external_url(block: &str) -> Option<String> {
    let mut search = 0;
    while let Some(idx) = block[search..].find("href=\"http") {
        let start = search + idx + 6; // skip href="
        if let Some(end) = block[start..].find('"') {
            let url = &block[start..start + end];
            if !url.contains("brave.com") && !url.contains("search.brave") {
                return Some(url.replace("&amp;", "&"));
            }
        }
        search = start + 1;
    }
    None
}

/// Extract text content of the first element whose class contains the given keyword.
fn extract_class_content(block: &str, class_keyword: &str) -> Option<String> {
    let pattern = format!("class=\"");
    let mut search = 0;
    while let Some(idx) = block[search..].find(&pattern) {
        let class_start = search + idx + 7;
        if let Some(class_end) = block[class_start..].find('"') {
            let class_val = &block[class_start..class_start + class_end];
            if class_val.contains(class_keyword) {
                // Find the > that closes this tag
                let tag_end = block[class_start + class_end..]
                    .find('>')
                    .map(|p| class_start + class_end + p + 1)?;
                // Find the closing </div> or </p>
                if let Some(close) = block[tag_end..].find("</") {
                    let content = &block[tag_end..tag_end + close];
                    let text = strip_html_tags(content).trim().to_string();
                    if !text.is_empty() {
                        return Some(text);
                    }
                }
            }
        }
        search = class_start + 1;
    }
    None
}

/// Extract text between two markers.
fn extract_between<'a>(s: &'a str, start: &str, end: &str) -> Option<&'a str> {
    let start_idx = s.find(start)? + start.len();
    let end_idx = s[start_idx..].find(end)? + start_idx;
    Some(&s[start_idx..end_idx])
}

/// Strip HTML tags from a string and decode common entities.
fn strip_html_tags(s: &str) -> String {
    let mut result = String::new();
    let mut in_tag = false;
    for ch in s.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => in_tag = false,
            _ if !in_tag => result.push(ch),
            _ => {}
        }
    }
    result
        .replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&quot;", "\"")
        .replace("&#x27;", "'")
        .replace("&nbsp;", " ")
        .trim()
        .to_string()
}
