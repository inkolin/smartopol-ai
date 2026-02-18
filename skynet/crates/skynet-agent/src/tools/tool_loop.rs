//! Tool execution loop — the core agentic behavior.
//!
//! Flow: prompt → LLM → if tool_use → execute tools → inject results → LLM → repeat
//! Stops when: stop_reason is not "tool_use", max iterations reached, or error.

use tracing::{debug, info, warn};

use crate::provider::{ChatRequest, ChatResponse, LlmProvider, ToolCall};

use super::{Tool, ToolResult};

/// Maximum tool loop iterations to prevent runaway agents.
const MAX_ITERATIONS: usize = 25;

/// Run the full tool execution loop (non-streaming).
///
/// Starts from `initial_request`, which must have `messages` or `raw_messages` set.
/// Returns the final `ChatResponse` (the one with `stop_reason != "tool_use"`).
pub async fn run_tool_loop(
    provider: &dyn LlmProvider,
    initial_request: ChatRequest,
    tools: &[Box<dyn Tool>],
) -> Result<ChatResponse, crate::provider::ProviderError> {
    // Build initial raw JSON message list from the structured messages.
    let mut raw_messages: Vec<serde_json::Value> =
        if let Some(ref raw) = initial_request.raw_messages {
            raw.clone()
        } else {
            initial_request
                .messages
                .iter()
                .map(|m| serde_json::json!({ "role": m.role, "content": m.content }))
                .collect()
        };

    let mut last_response: Option<ChatResponse> = None;

    for iteration in 0..MAX_ITERATIONS {
        // Build the request for this iteration, injecting the full message history.
        let mut req = initial_request.clone();
        req.raw_messages = Some(raw_messages.clone());

        debug!(iteration, "tool loop iteration");

        let response = provider.send(&req).await?;

        if response.tool_calls.is_empty() || response.stop_reason != "tool_use" {
            info!(iteration, "tool loop complete — no more tool calls");
            return Ok(response);
        }

        // Build the assistant turn content block list.
        // It includes any text content plus the tool_use blocks.
        let mut assistant_content: Vec<serde_json::Value> = Vec::new();

        if !response.content.is_empty() {
            assistant_content.push(serde_json::json!({
                "type": "text",
                "text": response.content,
            }));
        }

        for call in &response.tool_calls {
            assistant_content.push(serde_json::json!({
                "type": "tool_use",
                "id": call.id,
                "name": call.name,
                "input": call.input,
            }));
        }

        // Append the assistant message.
        raw_messages.push(serde_json::json!({
            "role": "assistant",
            "content": assistant_content,
        }));

        // Execute each tool call and collect results.
        let mut tool_result_content: Vec<serde_json::Value> = Vec::new();

        for call in &response.tool_calls {
            let result = execute_tool(tools, call).await;
            tool_result_content.push(serde_json::json!({
                "type": "tool_result",
                "tool_use_id": call.id,
                "content": result.content,
                "is_error": result.is_error,
            }));
        }

        // Append the user message containing all tool results.
        raw_messages.push(serde_json::json!({
            "role": "user",
            "content": tool_result_content,
        }));

        last_response = Some(response);
    }

    warn!(
        max_iterations = MAX_ITERATIONS,
        "tool loop hit maximum iterations"
    );

    // If we have a last response use that, otherwise return an error.
    if let Some(resp) = last_response {
        Ok(resp)
    } else {
        Err(crate::provider::ProviderError::Parse(format!(
            "tool loop exceeded {MAX_ITERATIONS} iterations without a final response"
        )))
    }
}

/// Find and execute the named tool. Returns an error ToolResult if not found.
async fn execute_tool(tools: &[Box<dyn Tool>], call: &ToolCall) -> ToolResult {
    match tools.iter().find(|t| t.name() == call.name) {
        Some(tool) => {
            debug!(tool = %call.name, "executing tool");
            tool.execute(call.input.clone()).await
        }
        None => ToolResult::error(format!("unknown tool: {}", call.name)),
    }
}
