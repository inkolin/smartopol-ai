//! Embed parsing — extracts Discord embeds from LLM output.
//!
//! The LLM can include a `DISCORD_EMBED:` sentinel block in its response to
//! have the bot send a rich embed. Format:
//!
//! ```text
//! DISCORD_EMBED:
//! title: My Title
//! color: #3498db
//! description: Some description
//! field: Name | Value | true
//! footer: Some footer text
//! ```
//!
//! The embed block ends at a blank line or end of text. Remaining text
//! outside the block is sent as normal chunked messages.

use serenity::builder::{CreateEmbed, CreateEmbedFooter};

/// A parsed embed extracted from LLM output.
pub struct ParsedEmbed {
    pub title: Option<String>,
    pub description: Option<String>,
    pub color: Option<u32>,
    pub fields: Vec<(String, String, bool)>,
    pub footer: Option<String>,
}

/// Try to extract a `DISCORD_EMBED:` block from the given text.
///
/// Returns `Some((embed, remaining_text))` if found, or `None` if no
/// embed sentinel is present.
pub fn try_parse_embed(text: &str) -> Option<(ParsedEmbed, String)> {
    let marker = "DISCORD_EMBED:";
    let start = text.find(marker)?;

    let embed_start = start + marker.len();
    let after_marker = &text[embed_start..];

    // The embed block ends at a double newline or end of text.
    let embed_end = after_marker.find("\n\n").unwrap_or(after_marker.len());
    let embed_block = after_marker[..embed_end].trim();

    let mut embed = ParsedEmbed {
        title: None,
        description: None,
        color: None,
        fields: Vec::new(),
        footer: None,
    };

    for line in embed_block.lines() {
        let line = line.trim();
        if let Some(val) = line.strip_prefix("title:") {
            embed.title = Some(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("color:") {
            let hex = val.trim().trim_start_matches('#');
            embed.color = u32::from_str_radix(hex, 16).ok();
        } else if let Some(val) = line.strip_prefix("description:") {
            embed.description = Some(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("footer:") {
            embed.footer = Some(val.trim().to_string());
        } else if let Some(val) = line.strip_prefix("field:") {
            let parts: Vec<&str> = val.splitn(3, '|').collect();
            if parts.len() >= 2 {
                let name = parts[0].trim().to_string();
                let value = parts[1].trim().to_string();
                let inline = parts
                    .get(2)
                    .map(|s| s.trim().eq_ignore_ascii_case("true"))
                    .unwrap_or(false);
                embed.fields.push((name, value, inline));
            }
        }
    }

    // Build remaining text: everything before and after the embed block.
    let before = text[..start].trim();
    let after = if embed_end < after_marker.len() {
        after_marker[embed_end..].trim()
    } else {
        ""
    };
    let remaining = match (before.is_empty(), after.is_empty()) {
        (true, true) => String::new(),
        (true, false) => after.to_string(),
        (false, true) => before.to_string(),
        (false, false) => format!("{}\n{}", before, after),
    };

    Some((embed, remaining))
}

impl ParsedEmbed {
    /// Convert to a serenity `CreateEmbed` builder.
    pub fn to_create_embed(&self) -> CreateEmbed {
        let mut e = CreateEmbed::new();
        if let Some(ref t) = self.title {
            e = e.title(t);
        }
        if let Some(ref d) = self.description {
            e = e.description(d);
        }
        if let Some(c) = self.color {
            e = e.colour(c);
        }
        for (name, value, inline) in &self.fields {
            e = e.field(name, value, *inline);
        }
        if let Some(ref f) = self.footer {
            e = e.footer(CreateEmbedFooter::new(f));
        }
        e
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_basic_embed() {
        let text = "Here is some info:\nDISCORD_EMBED:\ntitle: Status Report\ncolor: #3498db\ndescription: All systems operational\nfield: Uptime | 99.9% | true\nfooter: Skynet v0.5\n\nAnd here is more text.";
        let (embed, remaining) = try_parse_embed(text).unwrap();
        assert_eq!(embed.title.as_deref(), Some("Status Report"));
        assert_eq!(embed.color, Some(0x3498db));
        assert_eq!(
            embed.description.as_deref(),
            Some("All systems operational")
        );
        assert_eq!(embed.fields.len(), 1);
        assert_eq!(embed.fields[0].0, "Uptime");
        assert_eq!(embed.fields[0].1, "99.9%");
        assert!(embed.fields[0].2);
        assert_eq!(embed.footer.as_deref(), Some("Skynet v0.5"));
        assert!(remaining.contains("Here is some info:"));
        assert!(remaining.contains("And here is more text."));
    }

    #[test]
    fn no_embed_returns_none() {
        let text = "Just normal text without any embed.";
        assert!(try_parse_embed(text).is_none());
    }
}
