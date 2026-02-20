//! Cross-channel messaging tool — lets the AI send messages to any connected channel.

use std::sync::Arc;

use async_trait::async_trait;

use crate::pipeline::context::MessageContext;

use super::{Tool, ToolResult};

/// Tool that sends a message to a specific channel (Discord, terminal, etc.).
pub struct SendMessageTool<C: MessageContext + 'static> {
    ctx: Arc<C>,
}

impl<C: MessageContext + 'static> SendMessageTool<C> {
    pub fn new(ctx: Arc<C>) -> Self {
        Self { ctx }
    }
}

#[async_trait]
impl<C: MessageContext + 'static> Tool for SendMessageTool<C> {
    fn name(&self) -> &str {
        "send_message"
    }

    fn description(&self) -> &str {
        "Send a message to a connected channel (e.g. Discord, terminal). \
         Use `connected_channels` from the system prompt to see available targets."
    }

    fn input_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "channel": {
                    "type": "string",
                    "description": "Target channel name (e.g. 'discord', 'terminal'). Must be one of the connected channels."
                },
                "recipient": {
                    "type": "string",
                    "description": "Channel-specific target: Discord channel ID, session key for terminal, etc."
                },
                "message": {
                    "type": "string",
                    "description": "The text message to send."
                }
            },
            "required": ["channel", "recipient", "message"]
        })
    }

    async fn execute(&self, input: serde_json::Value) -> ToolResult {
        let channel = match input.get("channel").and_then(|v| v.as_str()) {
            Some(c) if !c.is_empty() => c,
            _ => return ToolResult::error("missing or empty 'channel' parameter"),
        };
        let recipient = match input.get("recipient").and_then(|v| v.as_str()) {
            Some(r) if !r.is_empty() => r,
            _ => return ToolResult::error("missing or empty 'recipient' parameter"),
        };
        let message = match input.get("message").and_then(|v| v.as_str()) {
            Some(m) if !m.is_empty() => m,
            _ => return ToolResult::error("missing or empty 'message' parameter"),
        };

        // Verify the channel is connected.
        let connected = self.ctx.connected_channels();
        if !connected.iter().any(|c| c == channel) {
            return ToolResult::error(format!(
                "channel '{}' is not connected. Available: {}",
                channel,
                connected.join(", ")
            ));
        }

        match self.ctx.send_to_channel(channel, recipient, message) {
            Ok(()) => ToolResult::success(format!(
                "Message sent to {} (recipient: {})",
                channel, recipient
            )),
            Err(e) => ToolResult::error(e),
        }
    }
}
