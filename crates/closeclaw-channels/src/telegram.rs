use closeclaw_core::types::{ChannelId, Event, Message, MessageContent, Sender, SessionId};
use closeclaw_gateway::hub::Hub;
use regex::Regex;
use std::path::PathBuf;
use std::sync::Arc;
use teloxide::net::Download;
use teloxide::prelude::*;
use teloxide::types::{InputFile, ParseMode};
use tokio::io::AsyncWriteExt;
use tracing::{error, info, warn};

/// Maximum message length allowed by the Telegram Bot API.
const TELEGRAM_MAX_LEN: usize = 4096;

/// Telegram bot channel — receives messages via long polling, sends responses back.
pub struct TelegramChannel {
    token: String,
    workspace: PathBuf,
}

impl TelegramChannel {
    pub fn new(token: String, workspace: PathBuf) -> Self {
        Self { token, workspace }
    }

    /// Run the Telegram bot with long polling. This blocks until the bot is stopped.
    pub async fn run(self, hub: Arc<Hub>) {
        let bot = Bot::new(&self.token);
        let downloads_dir = self.workspace.join("downloads");

        info!("Telegram channel starting (long polling)");

        let channel_id = ChannelId("telegram".to_string());

        teloxide::repl(bot, move |bot: Bot, telegram_msg: teloxide::types::Message| {
            let hub = hub.clone();
            let channel_id = channel_id.clone();
            let downloads_dir = downloads_dir.clone();
            async move {
                let text = if let Some(t) = telegram_msg.text() {
                    t.to_string()
                } else if let Some(photos) = telegram_msg.photo() {
                    match download_photo(&bot, photos, &downloads_dir).await {
                        Ok(path) => {
                            let caption = telegram_msg.caption().unwrap_or("");
                            format!("[Photo received and saved to {}]\n{}", path.display(), caption)
                        }
                        Err(e) => {
                            error!("Failed to download photo: {e}");
                            return Ok(());
                        }
                    }
                } else if let Some(doc) = telegram_msg.document() {
                    match download_document(&bot, doc, &downloads_dir).await {
                        Ok((path, filename)) => {
                            let caption = telegram_msg.caption().unwrap_or("");
                            format!("[File \"{filename}\" received and saved to {}]\n{}", path.display(), caption)
                        }
                        Err(e) => {
                            error!("Failed to download document: {e}");
                            return Ok(());
                        }
                    }
                } else {
                    return Ok(()); // ignore stickers, voice, etc.
                };

                let chat_id = telegram_msg.chat.id;
                let peer_id = format!("tg:{}", chat_id.0);
                let user_name = telegram_msg
                    .from
                    .as_ref()
                    .and_then(|u| u.username.clone())
                    .unwrap_or_else(|| "TelegramUser".to_string());

                let msg_id = uuid::Uuid::new_v4().to_string();
                let msg = Message {
                    id: msg_id.clone(),
                    session_id: SessionId(String::new()), // Hub router will assign
                    channel_id,
                    sender: Sender::User {
                        name: user_name,
                        id: peer_id,
                    },
                    content: MessageContent::Text(text),
                    timestamp: chrono::Utc::now(),
                };

                // Subscribe to events before spawning so we don't miss any
                let mut event_rx = hub.subscribe_events();

                // Spawn handle_message in background with oneshot for the final result
                let (result_tx, mut result_rx) = tokio::sync::oneshot::channel();
                let hub_bg = hub.clone();
                tokio::spawn(async move {
                    let result = hub_bg.handle_message(msg).await;
                    let _ = result_tx.send(result);
                });

                // Send typing indicator while the agent starts processing
                let _ = bot
                    .send_chat_action(chat_id, teloxide::types::ChatAction::Typing)
                    .await;

                // Streaming state
                let mut session_id: Option<SessionId> = None;
                let mut accumulated_text = String::new();
                let mut tool_lines: Vec<String> = Vec::new();
                let mut telegram_msg_id: Option<teloxide::types::MessageId> = None;
                let mut last_sent_len: usize = 0;
                let mut edit_interval =
                    tokio::time::interval(std::time::Duration::from_secs(2));
                edit_interval.tick().await; // consume the immediate first tick

                loop {
                    tokio::select! {
                        event = event_rx.recv() => {
                            let event = match event {
                                Ok(e) => e,
                                Err(_) => continue,
                            };
                            match &event {
                                Event::MessageReceived(m) if m.id == msg_id => {
                                    session_id = Some(m.session_id.clone());
                                }
                                Event::TextDelta { session_id: sid, text: delta }
                                    if session_id.as_ref() == Some(sid) =>
                                {
                                    accumulated_text.push_str(delta);
                                }
                                Event::ToolInvoked { session_id: sid, tool, .. }
                                    if session_id.as_ref() == Some(sid) =>
                                {
                                    tool_lines.push(format!("\n🔧 Using {}...", tool));
                                }
                                Event::ToolResult { session_id: sid, tool, .. }
                                    if session_id.as_ref() == Some(sid) =>
                                {
                                    if let Some(line) = tool_lines.iter_mut().find(
                                        |l| l.contains(&format!("Using {}...", tool)),
                                    ) {
                                        *line = format!("\n✓ {} done", tool);
                                    }
                                }
                                _ => {}
                            }
                        }

                        _ = edit_interval.tick() => {
                            let mut display = accumulated_text.clone();
                            for line in &tool_lines {
                                display.push_str(line);
                            }

                            if display.len() > last_sent_len
                                && !display.trim().is_empty()
                            {
                                let display_text = if display.len() > TELEGRAM_MAX_LEN {
                                    &display[..TELEGRAM_MAX_LEN]
                                } else {
                                    display.as_str()
                                };

                                match telegram_msg_id {
                                    None => {
                                        match bot.send_message(chat_id, display_text).await {
                                            Ok(sent) => {
                                                telegram_msg_id = Some(sent.id);
                                                last_sent_len = display.len();
                                            }
                                            Err(e) => {
                                                warn!("Failed to send streaming message: {e}");
                                            }
                                        }
                                    }
                                    Some(mid) => {
                                        match bot
                                            .edit_message_text(chat_id, mid, display_text)
                                            .await
                                        {
                                            Ok(_) => {
                                                last_sent_len = display.len();
                                            }
                                            Err(e) => {
                                                warn!("Failed to edit streaming message: {e}");
                                            }
                                        }
                                    }
                                }
                            }
                        }

                        result = &mut result_rx => {
                            let response = match result {
                                Ok(Ok(r)) => r,
                                Ok(Err(e)) => {
                                    error!("Agent error for chat {chat_id}: {e}");
                                    format!("Error: {e}")
                                }
                                Err(_) => "Internal error".to_string(),
                            };

                            let (image_paths, remaining_text) =
                                extract_image_paths(&response).await;

                            for path in &image_paths {
                                if let Err(e) = bot
                                    .send_photo(chat_id, InputFile::file(path))
                                    .await
                                {
                                    error!("Failed to send photo {}: {e}", path.display());
                                }
                            }

                            let text_to_send = remaining_text.trim();
                            if !text_to_send.is_empty() {
                                if let Some(mid) = telegram_msg_id {
                                    let html = markdown_to_telegram_html(text_to_send);
                                    let chunks = split_message(&html);

                                    let edit_result = bot
                                        .edit_message_text(chat_id, mid, chunks[0])
                                        .parse_mode(ParseMode::Html)
                                        .await;
                                    if let Err(e) = edit_result {
                                        warn!("HTML edit failed, falling back to plain text: {e}");
                                        let _ = bot
                                            .edit_message_text(chat_id, mid, chunks[0])
                                            .await;
                                    }

                                    for chunk in &chunks[1..] {
                                        let result = bot
                                            .send_message(chat_id, *chunk)
                                            .parse_mode(ParseMode::Html)
                                            .await;
                                        if let Err(e) = result {
                                            warn!("HTML send failed, falling back to plain text: {e}");
                                            if let Err(e2) =
                                                bot.send_message(chat_id, *chunk).await
                                            {
                                                error!("Failed to send Telegram message to {chat_id}: {e2}");
                                                break;
                                            }
                                        }
                                    }
                                } else {
                                    send_html(&bot, chat_id, text_to_send).await;
                                }
                            } else if let Some(mid) = telegram_msg_id {
                                let _ = bot.delete_message(chat_id, mid).await;
                            }

                            break;
                        }
                    }
                }

                Ok(())
            }
        })
        .await;
    }
}

/// Download the largest photo from a photo array and save it to the downloads directory.
async fn download_photo(
    bot: &Bot,
    photos: &[teloxide::types::PhotoSize],
    downloads_dir: &PathBuf,
) -> anyhow::Result<PathBuf> {
    let photo = photos.last().ok_or_else(|| anyhow::anyhow!("Empty photo array"))?;
    let file = bot.get_file(&photo.file.id).await?;
    let file_path = &file.path;

    tokio::fs::create_dir_all(downloads_dir).await?;
    let ext = std::path::Path::new(file_path)
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("jpg");
    let local_name = format!("{}.{}", uuid::Uuid::new_v4(), ext);
    let dest = downloads_dir.join(&local_name);

    let mut out = tokio::fs::File::create(&dest).await?;
    bot.download_file(file_path, &mut out).await?;
    out.flush().await?;

    info!("Downloaded photo to {}", dest.display());
    Ok(dest)
}

/// Download a document and save it to the downloads directory, preserving the original filename.
async fn download_document(
    bot: &Bot,
    doc: &teloxide::types::Document,
    downloads_dir: &PathBuf,
) -> anyhow::Result<(PathBuf, String)> {
    let file = bot.get_file(&doc.file.id).await?;
    let file_path = &file.path;

    tokio::fs::create_dir_all(downloads_dir).await?;
    let original_name = doc
        .file_name
        .clone()
        .unwrap_or_else(|| format!("{}.bin", uuid::Uuid::new_v4()));
    // Prepend UUID to avoid collisions while keeping the original name recognizable
    let local_name = format!("{}_{}", uuid::Uuid::new_v4(), original_name);
    let dest = downloads_dir.join(&local_name);

    let mut out = tokio::fs::File::create(&dest).await?;
    bot.download_file(file_path, &mut out).await?;
    out.flush().await?;

    info!("Downloaded document '{}' to {}", original_name, dest.display());
    Ok((dest, original_name))
}

/// Extract local image file paths from the response text.
///
/// Scans for absolute paths ending with common image extensions, verifies they
/// exist on disk, and returns the found paths plus the text with those paths stripped.
async fn extract_image_paths(text: &str) -> (Vec<PathBuf>, String) {
    let re = Regex::new(r#"(/[^\s\]\)"']+\.(?:jpg|jpeg|png|gif|webp|bmp))"#).unwrap();
    let mut paths = Vec::new();
    let mut cleaned = text.to_string();

    for cap in re.find_iter(text) {
        let path = PathBuf::from(cap.as_str());
        if tokio::fs::metadata(&path).await.is_ok() {
            paths.push(path);
            cleaned = cleaned.replace(cap.as_str(), "");
        }
    }

    (paths, cleaned)
}

/// Send a markdown response as Telegram HTML, falling back to plain text on parse errors.
pub async fn send_html(bot: &Bot, chat_id: teloxide::types::ChatId, markdown: &str) {
    let html = markdown_to_telegram_html(markdown);
    for chunk in split_message(&html) {
        let result = bot
            .send_message(chat_id, chunk)
            .parse_mode(ParseMode::Html)
            .await;
        match result {
            Ok(_) => {}
            Err(e) => {
                // If HTML parsing fails, retry as plain text
                warn!(chat_id = %chat_id, error = %e, "HTML send failed, falling back to plain text");
                if let Err(e2) = bot.send_message(chat_id, chunk).await {
                    error!("Failed to send Telegram message to {chat_id}: {e2}");
                    break;
                }
            }
        }
    }
}

/// Convert common markdown patterns to Telegram-compatible HTML.
///
/// Handles: bold, italic, inline code, code blocks, links, headers, and
/// horizontal rules. Unrecognized markdown passes through as-is.
pub fn markdown_to_telegram_html(md: &str) -> String {
    let mut out = String::with_capacity(md.len() + md.len() / 4);
    let mut in_code_block = false;

    for line in md.lines() {
        // Toggle fenced code blocks
        if line.trim_start().starts_with("```") {
            if in_code_block {
                out.push_str("</pre>");
                in_code_block = false;
            } else {
                // Strip optional language tag
                out.push_str("<pre>");
                in_code_block = true;
            }
            out.push('\n');
            continue;
        }

        if in_code_block {
            out.push_str(&escape_html(line));
            out.push('\n');
            continue;
        }

        // Horizontal rules
        let trimmed = line.trim();
        if (trimmed.starts_with("---") || trimmed.starts_with("***") || trimmed.starts_with("___"))
            && trimmed.chars().all(|c| c == '-' || c == '*' || c == '_' || c == ' ')
            && trimmed.len() >= 3
        {
            out.push_str("———\n");
            continue;
        }

        // Headers → bold
        if let Some(header_text) = strip_header(trimmed) {
            out.push_str("<b>");
            out.push_str(&convert_inline(&escape_html(header_text)));
            out.push_str("</b>\n");
            continue;
        }

        // Regular line: escape HTML entities first, then convert inline markdown
        let escaped = escape_html(line);
        let converted = convert_inline(&escaped);
        out.push_str(&converted);
        out.push('\n');
    }

    // Close unclosed code block
    if in_code_block {
        out.push_str("</pre>\n");
    }

    // Trim trailing newline
    while out.ends_with('\n') {
        out.pop();
    }

    out
}

/// Strip markdown header prefix (# through ######), returning the text after it.
fn strip_header(line: &str) -> Option<&str> {
    let trimmed = line.trim_start();
    if !trimmed.starts_with('#') {
        return None;
    }
    let hashes = trimmed.bytes().take_while(|&b| b == b'#').count();
    if hashes > 6 {
        return None;
    }
    let rest = &trimmed[hashes..];
    if rest.is_empty() || rest.starts_with(' ') {
        Some(rest.trim())
    } else {
        None
    }
}

/// Escape HTML special characters for Telegram.
fn escape_html(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
}

/// Convert inline markdown (bold, italic, code, links) to HTML.
/// Expects input that is already HTML-escaped.
fn convert_inline(s: &str) -> String {
    let mut result = s.to_string();

    // Inline code: `text` → <code>text</code>
    result = convert_delimited(&result, '`', "code");

    // Bold: **text** → <b>text</b>
    result = convert_double_delimited(&result, "**", "b");

    // Bold: __text__ → <b>text</b>
    result = convert_double_delimited(&result, "__", "b");

    // Italic: *text* → <i>text</i> (only single *, not **)
    result = convert_single_star_italic(&result);

    // Links: [text](url) → <a href="url">text</a>
    result = convert_links(&result);

    result
}

/// Convert `delimited` text to <tag>text</tag> for single-char delimiters.
fn convert_delimited(s: &str, delim: char, tag: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut open = false;

    while let Some(c) = chars.next() {
        if c == delim {
            if open {
                result.push_str(&format!("</{tag}>"));
                open = false;
            } else {
                result.push_str(&format!("<{tag}>"));
                open = true;
            }
        } else {
            result.push(c);
        }
    }

    // If unclosed, revert — put delimiter back
    if open {
        // Unclosed delimiter; just return original
        return s.to_string();
    }

    result
}

/// Convert **delimited** or __delimited__ text to <tag>text</tag>.
fn convert_double_delimited(s: &str, delim: &str, tag: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut remaining = s;

    loop {
        match remaining.find(delim) {
            Some(start) => {
                result.push_str(&remaining[..start]);
                let after_open = &remaining[start + delim.len()..];
                match after_open.find(delim) {
                    Some(end) => {
                        result.push_str(&format!("<{tag}>{}</{tag}>", &after_open[..end]));
                        remaining = &after_open[end + delim.len()..];
                    }
                    None => {
                        // No closing delimiter, keep as-is
                        result.push_str(&remaining[start..]);
                        return result;
                    }
                }
            }
            None => {
                result.push_str(remaining);
                return result;
            }
        }
    }
}

/// Convert single *text* to <i>text</i>, careful not to match **bold**.
fn convert_single_star_italic(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let bytes = s.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i] == b'*' {
            // Skip if this is part of ** (already handled)
            if i + 1 < bytes.len() && bytes[i + 1] == b'*' {
                result.push('*');
                result.push('*');
                i += 2;
                continue;
            }
            // Find closing single *
            if let Some(end) = s[i + 1..].find(|c: char| c == '*') {
                let inner = &s[i + 1..i + 1 + end];
                // Make sure closing * is not part of **
                if !inner.is_empty()
                    && (i + 1 + end + 1 >= bytes.len() || bytes[i + 1 + end + 1] != b'*')
                {
                    result.push_str(&format!("<i>{inner}</i>"));
                    i = i + 1 + end + 1;
                    continue;
                }
            }
            result.push('*');
            i += 1;
        } else {
            result.push(s[i..].chars().next().unwrap());
            i += s[i..].chars().next().unwrap().len_utf8();
        }
    }

    result
}

/// Convert [text](url) to <a href="url">text</a>.
fn convert_links(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut remaining = s;

    while let Some(bracket_start) = remaining.find('[') {
        result.push_str(&remaining[..bracket_start]);
        let after_bracket = &remaining[bracket_start + 1..];

        if let Some(bracket_end) = after_bracket.find(']') {
            let link_text = &after_bracket[..bracket_end];
            let after_close = &after_bracket[bracket_end + 1..];

            if after_close.starts_with('(') {
                if let Some(paren_end) = after_close.find(')') {
                    let url = &after_close[1..paren_end];
                    result.push_str(&format!("<a href=\"{url}\">{link_text}</a>"));
                    remaining = &after_close[paren_end + 1..];
                    continue;
                }
            }
            // Not a valid link, keep as-is
            result.push('[');
            remaining = after_bracket;
        } else {
            result.push('[');
            remaining = after_bracket;
        }
    }

    result.push_str(remaining);
    result
}

/// Split a message into chunks that fit within Telegram's 4096-character limit.
/// Splits at newline boundaries when possible, otherwise hard-splits.
pub fn split_message(text: &str) -> Vec<&str> {
    if text.len() <= TELEGRAM_MAX_LEN {
        return vec![text];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while !remaining.is_empty() {
        if remaining.len() <= TELEGRAM_MAX_LEN {
            chunks.push(remaining);
            break;
        }

        // Try to find a newline boundary within the limit
        let split_at = match remaining[..TELEGRAM_MAX_LEN].rfind('\n') {
            Some(pos) if pos > 0 => pos + 1, // include the newline in current chunk
            _ => TELEGRAM_MAX_LEN,            // hard split
        };

        chunks.push(&remaining[..split_at]);
        remaining = &remaining[split_at..];
    }

    chunks
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_split_short_message() {
        let msg = "Hello, world!";
        let chunks = split_message(msg);
        assert_eq!(chunks, vec!["Hello, world!"]);
    }

    #[test]
    fn test_split_at_newline_boundary() {
        let line = "x".repeat(100);
        let mut msg = String::new();
        for _ in 0..50 {
            msg.push_str(&line);
            msg.push('\n');
        }

        let chunks = split_message(&msg);
        assert!(chunks.len() >= 2);
        assert!(chunks[0].len() <= TELEGRAM_MAX_LEN);
        assert!(chunks[0].ends_with('\n'));
        let reassembled: String = chunks.concat();
        assert_eq!(reassembled, msg);
    }

    #[test]
    fn test_split_no_newlines() {
        let msg = "a".repeat(5000);
        let chunks = split_message(&msg);
        assert_eq!(chunks.len(), 2);
        assert_eq!(chunks[0].len(), TELEGRAM_MAX_LEN);
        assert_eq!(chunks[1].len(), 5000 - TELEGRAM_MAX_LEN);
    }

    #[test]
    fn test_split_empty() {
        let chunks = split_message("");
        assert_eq!(chunks, vec![""]);
    }

    // --- Markdown to HTML tests ---

    #[test]
    fn test_bold() {
        assert_eq!(
            markdown_to_telegram_html("this is **bold** text"),
            "this is <b>bold</b> text"
        );
    }

    #[test]
    fn test_italic() {
        assert_eq!(
            markdown_to_telegram_html("this is *italic* text"),
            "this is <i>italic</i> text"
        );
    }

    #[test]
    fn test_inline_code() {
        assert_eq!(
            markdown_to_telegram_html("use `web_search` tool"),
            "use <code>web_search</code> tool"
        );
    }

    #[test]
    fn test_code_block() {
        let md = "before\n```rust\nfn main() {}\n```\nafter";
        let html = markdown_to_telegram_html(md);
        assert!(html.contains("<pre>"));
        assert!(html.contains("fn main() {}"));
        assert!(html.contains("</pre>"));
    }

    #[test]
    fn test_headers() {
        assert_eq!(
            markdown_to_telegram_html("# Title"),
            "<b>Title</b>"
        );
        assert_eq!(
            markdown_to_telegram_html("## Subtitle"),
            "<b>Subtitle</b>"
        );
    }

    #[test]
    fn test_link() {
        assert_eq!(
            markdown_to_telegram_html("[click here](https://example.com)"),
            "<a href=\"https://example.com\">click here</a>"
        );
    }

    #[test]
    fn test_html_entities_escaped() {
        assert_eq!(
            markdown_to_telegram_html("a < b & c > d"),
            "a &lt; b &amp; c &gt; d"
        );
    }

    #[test]
    fn test_mixed_formatting() {
        let md = "# Weather Report\n\n**Temperature**: 72°F\n*Partly cloudy*\n\nUse `web_search` for more.";
        let html = markdown_to_telegram_html(md);
        assert!(html.contains("<b>Weather Report</b>"));
        assert!(html.contains("<b>Temperature</b>: 72°F"));
        assert!(html.contains("<i>Partly cloudy</i>"));
        assert!(html.contains("<code>web_search</code>"));
    }

    #[test]
    fn test_horizontal_rule() {
        let html = markdown_to_telegram_html("above\n---\nbelow");
        assert!(html.contains("———"));
    }
}
