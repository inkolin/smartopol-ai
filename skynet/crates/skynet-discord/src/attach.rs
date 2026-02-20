//! Attachment handling — classifies Discord attachments and converts them
//! into Anthropic-style content blocks for the LLM pipeline.

use base64::Engine;
use serde_json::Value;
use serenity::model::channel::Attachment;
use tracing::warn;

/// Attachment classification by MIME type.
pub enum AttachmentKind {
    Image,
    Text,
    Voice,
    Audio,
    Other,
}

/// Classify a Discord attachment by its content type and filename.
pub fn classify(attachment: &Attachment) -> AttachmentKind {
    let ct = attachment.content_type.as_deref().unwrap_or("");
    if ct.starts_with("image/") {
        AttachmentKind::Image
    } else if ct.starts_with("text/") || is_text_extension(&attachment.filename) {
        AttachmentKind::Text
    } else if ct == "audio/ogg" && attachment.filename.ends_with(".ogg") {
        // Discord voice messages are OGG files.
        AttachmentKind::Voice
    } else if ct.starts_with("audio/") {
        AttachmentKind::Audio
    } else {
        AttachmentKind::Other
    }
}

/// Convert a slice of Discord attachments into Anthropic content blocks.
///
/// Images become `{"type":"image","source":{"type":"base64",...}}` blocks.
/// Text files become `{"type":"text",...}` blocks with the file content.
/// Voice/audio/other produce placeholder text blocks.
pub async fn to_content_blocks(attachments: &[Attachment], max_bytes: u64) -> Vec<Value> {
    let mut blocks = Vec::new();

    for att in attachments {
        if u64::from(att.size) > max_bytes {
            blocks.push(serde_json::json!({
                "type": "text",
                "text": format!("[Attachment '{}' skipped: {} bytes exceeds limit]", att.filename, att.size)
            }));
            continue;
        }

        match classify(att) {
            AttachmentKind::Image => match download_bytes(&att.url).await {
                Ok(bytes) => {
                    let b64 = base64::engine::general_purpose::STANDARD.encode(&bytes);
                    let media_type = att.content_type.as_deref().unwrap_or("image/png");
                    blocks.push(serde_json::json!({
                        "type": "image",
                        "source": {
                            "type": "base64",
                            "media_type": media_type,
                            "data": b64
                        }
                    }));
                }
                Err(e) => {
                    warn!(filename = %att.filename, error = %e, "failed to download image");
                    blocks.push(serde_json::json!({
                        "type": "text",
                        "text": format!("[Image '{}' download failed: {}]", att.filename, e)
                    }));
                }
            },
            AttachmentKind::Text => match download_text(&att.url).await {
                Ok(text) => {
                    blocks.push(serde_json::json!({
                        "type": "text",
                        "text": format!("--- {} ---\n{}", att.filename, text)
                    }));
                }
                Err(e) => {
                    warn!(filename = %att.filename, error = %e, "failed to download text file");
                    blocks.push(serde_json::json!({
                        "type": "text",
                        "text": format!("[File '{}' download failed: {}]", att.filename, e)
                    }));
                }
            },
            AttachmentKind::Voice => {
                blocks.push(serde_json::json!({
                    "type": "text",
                    "text": format!(
                        "[Voice message: '{}' ({} bytes) — use voice_transcription config to enable transcription]",
                        att.filename, att.size
                    )
                }));
            }
            AttachmentKind::Audio => {
                blocks.push(serde_json::json!({
                    "type": "text",
                    "text": format!("[Audio attachment: '{}' ({} bytes)]", att.filename, att.size)
                }));
            }
            AttachmentKind::Other => {
                let ct = att.content_type.as_deref().unwrap_or("unknown");
                blocks.push(serde_json::json!({
                    "type": "text",
                    "text": format!("[Attachment: '{}' ({}, {} bytes)]", att.filename, ct, att.size)
                }));
            }
        }
    }

    blocks
}

/// Download raw bytes from a voice attachment for transcription.
pub async fn download_voice_bytes(attachment: &Attachment) -> Result<Vec<u8>, String> {
    download_bytes(&attachment.url)
        .await
        .map_err(|e| e.to_string())
}

fn is_text_extension(filename: &str) -> bool {
    let lower = filename.to_lowercase();
    matches!(
        lower.rsplit('.').next(),
        Some(
            "txt"
                | "md"
                | "rs"
                | "py"
                | "js"
                | "ts"
                | "json"
                | "toml"
                | "yaml"
                | "yml"
                | "xml"
                | "html"
                | "css"
                | "csv"
                | "log"
                | "sh"
                | "bash"
                | "cfg"
                | "ini"
                | "conf"
                | "go"
                | "java"
                | "c"
                | "cpp"
                | "h"
                | "hpp"
                | "rb"
                | "sql"
                | "env"
        )
    )
}

async fn download_bytes(url: &str) -> Result<Vec<u8>, reqwest::Error> {
    let resp = reqwest::get(url).await?;
    resp.bytes().await.map(|b| b.to_vec())
}

async fn download_text(url: &str) -> Result<String, reqwest::Error> {
    reqwest::get(url).await?.text().await
}
