//! Message sending helpers for the Telegram adapter.
//!
//! Telegram's message limit is 4096 characters. We use 4090 for safety.
//! Tries MarkdownV2 first; falls back to plain text if Telegram rejects the parse mode.

use std::time::Duration;

use teloxide::prelude::*;
use teloxide::types::ParseMode;
use tracing::warn;

/// Maximum characters per Telegram message (limit is 4096; we use 4090 for safety).
const CHUNK_MAX: usize = 4090;

/// Code-fence-aware message splitter for Telegram.
///
/// Mirrors `skynet-discord/src/send.rs` logic but with a 4090-char limit.
/// When a split falls inside a fenced code block, the fence is closed before
/// the chunk boundary and re-opened at the start of the next chunk.
pub fn split_chunks_smart(text: &str) -> Vec<String> {
    if text.len() <= CHUNK_MAX {
        return vec![text.to_string()];
    }

    let lines: Vec<&str> = text.split('\n').collect();
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut fence_lang: Option<String> = None;

    for line in &lines {
        let cost = if current.is_empty() {
            line.len()
        } else {
            1 + line.len()
        };

        if !current.is_empty() && current.len() + cost > CHUNK_MAX {
            // Close any open fence before ending the chunk.
            if fence_lang.is_some() {
                current.push_str("\n```");
            }
            chunks.push(current);
            current = String::new();
            // Re-open the fence in the new chunk.
            if let Some(ref lang) = fence_lang {
                if lang.is_empty() {
                    current.push_str("```\n");
                } else {
                    current.push_str("```");
                    current.push_str(lang);
                    current.push('\n');
                }
            }
        }

        if !current.is_empty() {
            current.push('\n');
        }
        current.push_str(line);

        // Update fence tracking after appending.
        let trimmed = line.trim_start();
        if let Some(after_fence) = trimmed.strip_prefix("```") {
            if fence_lang.is_some() {
                fence_lang = None; // closing fence
            } else {
                fence_lang = Some(after_fence.trim().to_string()); // opening fence
            }
        }
    }

    if !current.is_empty() {
        chunks.push(current);
    }

    // Safety net: force-split any chunk that still exceeds CHUNK_MAX
    // (e.g. a single line longer than 4090 chars).
    let mut result = Vec::new();
    for chunk in chunks {
        if chunk.len() <= CHUNK_MAX {
            result.push(chunk);
        } else {
            let mut remaining = chunk.as_str();
            while remaining.len() > CHUNK_MAX {
                let split_at = remaining[..CHUNK_MAX]
                    .rfind('\n')
                    .or_else(|| remaining[..CHUNK_MAX].rfind(' '))
                    .unwrap_or(CHUNK_MAX);
                result.push(remaining[..split_at].to_string());
                remaining = remaining[split_at..].trim_start();
            }
            if !remaining.is_empty() {
                result.push(remaining.to_string());
            }
        }
    }

    result
}

/// Escape special characters for Telegram MarkdownV2.
///
/// MarkdownV2 requires escaping: `_ * [ ] ( ) ~ ` # + - = | { } . !`
pub fn escape_markdown_v2(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 16);
    for ch in text.chars() {
        match ch {
            '_' | '*' | '[' | ']' | '(' | ')' | '~' | '`' | '#' | '+' | '-' | '=' | '|'
            | '{' | '}' | '.' | '!' => {
                out.push('\\');
                out.push(ch);
            }
            _ => out.push(ch),
        }
    }
    out
}

/// Send `text` to `chat_id` in smart-chunked messages.
///
/// Tries MarkdownV2 first; if Telegram rejects the parse mode (e.g. bad escaping),
/// falls back to plain text for that chunk.
///
/// A 100ms delay is inserted between consecutive chunks to avoid hitting rate limits.
pub async fn send_response(bot: &Bot, chat_id: ChatId, text: &str) {
    let chunks = split_chunks_smart(text);
    for (i, chunk) in chunks.iter().enumerate() {
        let escaped = escape_markdown_v2(chunk);
        let sent = bot
            .send_message(chat_id, &escaped)
            .parse_mode(ParseMode::MarkdownV2)
            .await;

        if sent.is_err() {
            // MarkdownV2 rejected — fall back to plain text.
            if let Err(e) = bot.send_message(chat_id, chunk).await {
                warn!(error = %e, chunk_index = i, "Telegram: failed to send plain-text fallback");
            }
        }

        if i + 1 < chunks.len() {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_is_single_chunk() {
        let chunks = split_chunks_smart("Hello, world!");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "Hello, world!");
    }

    #[test]
    fn exactly_chunk_max_is_single_chunk() {
        let text = "a".repeat(CHUNK_MAX);
        let chunks = split_chunks_smart(&text);
        assert_eq!(chunks.len(), 1);
    }

    #[test]
    fn over_limit_splits_on_newline() {
        let line = "a".repeat(2000);
        let text = format!("{line}\n{line}\n{line}");
        let chunks = split_chunks_smart(&text);
        assert!(chunks.len() >= 2);
        for c in &chunks {
            assert!(c.len() <= CHUNK_MAX, "chunk too large: {}", c.len());
        }
    }

    #[test]
    fn very_long_single_line_force_splits() {
        let text = "x".repeat(9000);
        let chunks = split_chunks_smart(&text);
        assert!(chunks.len() >= 2);
        for c in &chunks {
            assert!(c.len() <= CHUNK_MAX);
        }
    }

    #[test]
    fn code_fence_preserved_across_chunks() {
        let mut text = String::from("Intro.\n```rust\n");
        // Each line is ~30 chars; 200 lines = ~6000 chars — exceeds CHUNK_MAX=4090.
        for i in 0..200 {
            text.push_str(&format!("let variable_name_{i:04} = {i:05}; // comment\n"));
        }
        text.push_str("```\nAfter fence.");

        let chunks = split_chunks_smart(&text);
        assert!(chunks.len() >= 2, "expected multiple chunks, got {}", chunks.len());
        for c in &chunks {
            assert!(c.len() <= CHUNK_MAX, "chunk too large: {}", c.len());
        }
    }

    #[test]
    fn code_fence_language_preserved() {
        let mut text = String::from("```python\n");
        // ~55 chars per line; 100 lines = ~5500 chars — exceeds CHUNK_MAX=4090.
        for _ in 0..100 {
            text.push_str("print('hello world this is a reasonably long line of python code')\n");
        }
        text.push_str("```\n");

        let chunks = split_chunks_smart(&text);
        assert!(chunks.len() >= 2);
        assert!(
            chunks[1].starts_with("```python"),
            "second chunk should reopen with ```python, got: {}",
            &chunks[1][..chunks[1].len().min(60)]
        );
    }

    #[test]
    fn escape_markdown_v2_escapes_specials() {
        let input = "Hello. World! (test) [link] ~strike~";
        let escaped = escape_markdown_v2(input);
        assert!(escaped.contains("\\."));
        assert!(escaped.contains("\\!"));
        assert!(escaped.contains("\\("));
        assert!(escaped.contains("\\)"));
        assert!(escaped.contains("\\["));
        assert!(escaped.contains("\\]"));
        assert!(escaped.contains("\\~"));
    }

    #[test]
    fn escape_markdown_v2_leaves_normal_chars() {
        let input = "Hello world 123 abc";
        assert_eq!(escape_markdown_v2(input), input);
    }
}
