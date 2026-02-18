use serde::{Deserialize, Serialize};
use std::fmt;

/// Controls how much token budget the model may spend on internal reasoning
/// before generating the visible response.
///
/// Each level maps to a `budget_tokens` cap sent to the Anthropic API.
/// `Off` disables the thinking feature entirely (no thinking block is added).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ThinkingLevel {
    /// Thinking disabled — no reasoning block is sent to the API.
    Off,
    /// Up to 1 024 tokens of internal reasoning.
    Minimal,
    /// Up to 4 096 tokens of internal reasoning.
    Low,
    /// Up to 8 192 tokens of internal reasoning.
    Medium,
    /// Up to 16 384 tokens of internal reasoning.
    High,
    /// Up to 32 768 tokens of internal reasoning.
    XHigh,
}

impl ThinkingLevel {
    /// Return the token budget that should be sent to the Anthropic API.
    /// Returns `0` for `Off` — callers should skip the thinking block entirely.
    pub fn budget_tokens(&self) -> u32 {
        match self {
            ThinkingLevel::Off => 0,
            ThinkingLevel::Minimal => 1_024,
            ThinkingLevel::Low => 4_096,
            ThinkingLevel::Medium => 8_192,
            ThinkingLevel::High => 16_384,
            ThinkingLevel::XHigh => 32_768,
        }
    }

    /// Parse from a string slug.  Case-insensitive.
    ///
    /// Accepted values: `"off"`, `"minimal"`, `"low"`, `"medium"`, `"high"`, `"xhigh"`.
    pub fn parse(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "off" => Some(ThinkingLevel::Off),
            "minimal" => Some(ThinkingLevel::Minimal),
            "low" => Some(ThinkingLevel::Low),
            "medium" => Some(ThinkingLevel::Medium),
            "high" => Some(ThinkingLevel::High),
            "xhigh" => Some(ThinkingLevel::XHigh),
            _ => None,
        }
    }
}

/// Remove thinking blocks from conversation messages before re-sending to the LLM.
/// Anthropic's API rejects requests that include thinking content blocks from previous turns.
/// The assistant's text content is preserved; only thinking/reasoning blocks are removed.
pub fn strip_thinking_blocks(messages: &mut [serde_json::Value]) {
    for msg in messages.iter_mut() {
        if msg.get("role").and_then(|r| r.as_str()) != Some("assistant") {
            continue;
        }
        // If content is an array of blocks, filter out thinking blocks
        if let Some(content) = msg.get_mut("content") {
            if let Some(blocks) = content.as_array() {
                let filtered: Vec<serde_json::Value> = blocks
                    .iter()
                    .filter(|block| block.get("type").and_then(|t| t.as_str()) != Some("thinking"))
                    .cloned()
                    .collect();
                *content = serde_json::Value::Array(filtered);
            }
        }
    }
}

/// Default is `Off` — thinking is opt-in per request.
impl Default for ThinkingLevel {
    fn default() -> Self {
        ThinkingLevel::Off
    }
}

impl fmt::Display for ThinkingLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            ThinkingLevel::Off => "off",
            ThinkingLevel::Minimal => "minimal",
            ThinkingLevel::Low => "low",
            ThinkingLevel::Medium => "medium",
            ThinkingLevel::High => "high",
            ThinkingLevel::XHigh => "xhigh",
        };
        f.write_str(label)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_tokens_values() {
        assert_eq!(ThinkingLevel::Off.budget_tokens(), 0);
        assert_eq!(ThinkingLevel::Minimal.budget_tokens(), 1_024);
        assert_eq!(ThinkingLevel::Low.budget_tokens(), 4_096);
        assert_eq!(ThinkingLevel::Medium.budget_tokens(), 8_192);
        assert_eq!(ThinkingLevel::High.budget_tokens(), 16_384);
        assert_eq!(ThinkingLevel::XHigh.budget_tokens(), 32_768);
    }

    #[test]
    fn from_str_all_variants() {
        for (input, expected) in [
            ("off", ThinkingLevel::Off),
            ("minimal", ThinkingLevel::Minimal),
            ("low", ThinkingLevel::Low),
            ("medium", ThinkingLevel::Medium),
            ("high", ThinkingLevel::High),
            ("xhigh", ThinkingLevel::XHigh),
            ("OFF", ThinkingLevel::Off),
            ("HIGH", ThinkingLevel::High),
        ] {
            assert_eq!(
                ThinkingLevel::parse(input),
                Some(expected),
                "input: {input}"
            );
        }
        assert_eq!(ThinkingLevel::parse("unknown"), None);
    }

    #[test]
    fn display_round_trips() {
        for level in [
            ThinkingLevel::Off,
            ThinkingLevel::Minimal,
            ThinkingLevel::Low,
            ThinkingLevel::Medium,
            ThinkingLevel::High,
            ThinkingLevel::XHigh,
        ] {
            let s = level.to_string();
            assert_eq!(ThinkingLevel::parse(&s), Some(level));
        }
    }

    #[test]
    fn default_is_off() {
        assert_eq!(ThinkingLevel::default(), ThinkingLevel::Off);
    }

    #[test]
    fn strip_removes_thinking_blocks() {
        let mut messages = vec![
            serde_json::json!({
                "role": "assistant",
                "content": [
                    { "type": "thinking", "thinking": "internal reasoning" },
                    { "type": "text", "text": "Hello!" }
                ]
            }),
            serde_json::json!({
                "role": "user",
                "content": "Hi"
            }),
        ];
        super::strip_thinking_blocks(&mut messages);
        let content = messages[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
    }

    #[test]
    fn strip_leaves_user_messages_unchanged() {
        let mut messages = vec![serde_json::json!({
            "role": "user",
            "content": [
                { "type": "text", "text": "Hello" }
            ]
        })];
        super::strip_thinking_blocks(&mut messages);
        let content = messages[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
    }

    #[test]
    fn strip_noop_when_no_thinking_blocks() {
        let mut messages = vec![serde_json::json!({
            "role": "assistant",
            "content": [
                { "type": "text", "text": "Sure, here is the answer." }
            ]
        })];
        super::strip_thinking_blocks(&mut messages);
        let content = messages[0]["content"].as_array().unwrap();
        assert_eq!(content.len(), 1);
        assert_eq!(content[0]["type"], "text");
    }

    #[test]
    fn strip_handles_string_content_untouched() {
        // When content is a plain string (not an array of blocks), nothing should crash.
        let mut messages = vec![serde_json::json!({
            "role": "assistant",
            "content": "plain text response"
        })];
        super::strip_thinking_blocks(&mut messages);
        assert_eq!(messages[0]["content"], "plain text response");
    }
}
