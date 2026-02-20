use serenity::builder::CreateMessage;
use serenity::model::channel::MessageReference;
use serenity::model::id::{ChannelId, MessageId};

/// Maximum characters per Discord message (2000 is the limit; we use 1950 for safety).
const CHUNK_MAX: usize = 1950;

/// Code-fence-aware message splitter.
///
/// Tracks whether we are inside a fenced code block (` ```lang `). When a
/// chunk boundary falls inside a fence the chunk is closed with ` ``` ` and
/// the next chunk is re-opened with ` ```lang `.  Splits always occur on
/// `\n` boundaries when possible.
pub fn split_chunks_smart(text: &str) -> Vec<String> {
    if text.len() <= CHUNK_MAX {
        return vec![text.to_string()];
    }

    let lines: Vec<&str> = text.split('\n').collect();
    let mut chunks: Vec<String> = Vec::new();
    let mut current = String::new();
    let mut fence_lang: Option<String> = None;

    for line in &lines {
        // How many bytes would adding this line cost?
        let cost = if current.is_empty() {
            line.len()
        } else {
            1 + line.len() // newline separator
        };

        // Would this line overflow the chunk?
        if !current.is_empty() && current.len() + cost > CHUNK_MAX {
            // Close an open fence before ending the chunk.
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

        // Update fence tracking *after* appending the line.
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
    // (e.g. a single line longer than 1950 chars).
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

/// Send `text` to `channel_id` in smart-chunked messages.
///
/// If `reply_to` is `Some`, the first chunk is sent as a reply to that message.
/// Subsequent chunks are sent as plain messages.
pub async fn send_response(
    http: &serenity::http::Http,
    channel_id: ChannelId,
    text: &str,
    reply_to: Option<MessageId>,
) -> Result<(), serenity::Error> {
    let chunks = split_chunks_smart(text);
    for (i, chunk) in chunks.iter().enumerate() {
        if i == 0 {
            if let Some(msg_id) = reply_to {
                let msg = CreateMessage::new()
                    .content(chunk)
                    .reference_message(MessageReference::from((channel_id, msg_id)));
                channel_id.send_message(http, msg).await?;
            } else {
                channel_id.say(http, chunk).await?;
            }
        } else {
            channel_id.say(http, chunk).await?;
        }
    }
    Ok(())
}

/// Legacy alias — send without reply-to.
pub async fn send_chunked(
    http: &serenity::http::Http,
    channel_id: ChannelId,
    text: &str,
) -> Result<(), serenity::Error> {
    send_response(http, channel_id, text, None).await
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
    fn long_text_splits_on_newline() {
        let line = "a".repeat(1000);
        let text = format!("{}\n{}", line, line);
        let chunks = split_chunks_smart(&text);
        assert_eq!(chunks.len(), 2);
        for c in &chunks {
            assert!(c.len() <= CHUNK_MAX, "chunk too large: {}", c.len());
        }
    }

    #[test]
    fn very_long_word_still_splits() {
        let text = "x".repeat(4000);
        let chunks = split_chunks_smart(&text);
        assert!(chunks.len() >= 2);
        for c in &chunks {
            assert!(c.len() <= CHUNK_MAX);
        }
    }

    #[test]
    fn code_fence_is_preserved_across_chunks() {
        // Build text that forces a split inside a code fence.
        let mut text = String::from("Some intro text.\n```rust\n");
        for i in 0..100 {
            text.push_str(&format!("let x_{} = {};\n", i, i));
        }
        text.push_str("```\nAfter the fence.");

        let chunks = split_chunks_smart(&text);

        // Every chunk should have balanced fences.
        for chunk in &chunks {
            let opens = chunk.matches("```").count();
            // Fences should be balanced (even count) or the chunk is self-contained.
            // The first chunk of a split fence gets a closing ```, the next gets an opening.
            // Each chunk should not have unmatched fences.
            assert!(
                opens % 2 == 0 || chunk.trim().ends_with("```"),
                "unbalanced fences in chunk: {}",
                &chunk[..chunk.len().min(200)]
            );
        }

        for c in &chunks {
            assert!(c.len() <= CHUNK_MAX, "chunk too large: {}", c.len());
        }
    }

    #[test]
    fn code_fence_language_preserved() {
        let mut text = String::from("```python\n");
        // Fill enough to force a split
        for _ in 0..100 {
            text.push_str("print('hello world this is a long line of code')\n");
        }
        text.push_str("```\n");

        let chunks = split_chunks_smart(&text);
        assert!(chunks.len() >= 2);

        // Second chunk should start with ```python
        if chunks.len() >= 2 {
            assert!(
                chunks[1].starts_with("```python"),
                "second chunk should reopen with ```python, got: {}",
                &chunks[1][..chunks[1].len().min(50)]
            );
        }
    }
}
