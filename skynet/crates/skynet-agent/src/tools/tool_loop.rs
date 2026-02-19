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
/// Returns the final `ChatResponse` and the deduplicated list of tool names called
/// during the loop. The caller uses the tool name list for transparent usage tracking.
pub async fn run_tool_loop(
    provider: &dyn LlmProvider,
    initial_request: ChatRequest,
    tools: &[Box<dyn Tool>],
) -> Result<(ChatResponse, Vec<String>), crate::provider::ProviderError> {
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
    // Collect every tool name called across all iterations.
    let mut called_tools: Vec<String> = Vec::new();

    for iteration in 0..MAX_ITERATIONS {
        let mut req = initial_request.clone();
        req.raw_messages = Some(raw_messages.clone());

        debug!(iteration, "tool loop iteration");

        let response = provider.send(&req).await?;

        if response.tool_calls.is_empty() || response.stop_reason != "tool_use" {
            info!(iteration, "tool loop complete — no more tool calls");
            return Ok((response, called_tools));
        }

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

        raw_messages.push(serde_json::json!({
            "role": "assistant",
            "content": assistant_content,
        }));

        let mut tool_result_content: Vec<serde_json::Value> = Vec::new();

        for call in &response.tool_calls {
            called_tools.push(call.name.clone());
            let result = execute_tool(tools, call).await;
            tool_result_content.push(serde_json::json!({
                "type": "tool_result",
                "tool_use_id": call.id,
                "content": result.content,
                "is_error": result.is_error,
            }));
        }

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

    if let Some(resp) = last_response {
        Ok((resp, called_tools))
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
