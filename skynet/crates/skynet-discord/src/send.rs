/// Maximum characters per Discord message (2000 is the limit; we use 1950 for safety).
const CHUNK_MAX: usize = 1950;

/// Split `text` into chunks of at most [`CHUNK_MAX`] characters, preferring
/// splits on whitespace/newline boundaries to avoid cutting words mid-way.
pub fn split_chunks(text: &str) -> Vec<String> {
    if text.len() <= CHUNK_MAX {
        return vec![text.to_string()];
    }

    let mut chunks = Vec::new();
    let mut remaining = text;

    while remaining.len() > CHUNK_MAX {
        // Try to split on the last newline within the window.
        let window = &remaining[..CHUNK_MAX];
        let split_at = window
            .rfind('\n')
            .or_else(|| window.rfind(' '))
            .unwrap_or(CHUNK_MAX);

        chunks.push(remaining[..split_at].to_string());
        remaining = remaining[split_at..].trim_start();
    }

    if !remaining.is_empty() {
        chunks.push(remaining.to_string());
    }

    chunks
}

/// Send `text` to `channel_id` in ≤1950-char chunks.
pub async fn send_chunked(
    http: &serenity::http::Http,
    channel_id: serenity::model::id::ChannelId,
    text: &str,
) -> Result<(), serenity::Error> {
    for chunk in split_chunks(text) {
        channel_id.say(http, &chunk).await?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_text_is_single_chunk() {
        let chunks = split_chunks("Hello, world!");
        assert_eq!(chunks.len(), 1);
        assert_eq!(chunks[0], "Hello, world!");
    }

    #[test]
    fn long_text_splits_on_newline() {
        let line = "a".repeat(1000);
        let text = format!("{}\n{}", line, line);
        let chunks = split_chunks(&text);
        // Both halves fit inside CHUNK_MAX, so should be 2 chunks.
        assert_eq!(chunks.len(), 2);
        for c in &chunks {
            assert!(c.len() <= CHUNK_MAX, "chunk too large: {}", c.len());
        }
    }

    #[test]
    fn very_long_word_still_splits() {
        let text = "x".repeat(4000);
        let chunks = split_chunks(&text);
        assert!(chunks.len() >= 2);
        for c in &chunks {
            assert!(c.len() <= CHUNK_MAX);
        }
    }
}
