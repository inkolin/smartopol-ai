//! Tool registry — builds the canonical tool list for any channel adapter.

use std::sync::Arc;

use crate::pipeline::context::MessageContext;
use crate::provider::ToolDefinition;

use super::bash_session::BashSessionTool;
use super::execute_command::ExecuteCommandTool;
use super::knowledge::{KnowledgeSearchTool, KnowledgeWriteTool};
use super::reminder::ReminderTool;
use super::{to_definitions, Tool};

/// Build the full list of tools available to the AI for a given request.
///
/// Includes:
/// - `read_file`, `write_file`, `list_files`, `search_files` (filesystem, skynet-agent)
/// - `execute_command` (one-shot sh -c via TerminalManager)
/// - `bash` (persistent PTY bash session via TerminalManager)
/// - `reminder` (schedule proactive reminders via the scheduler)
///
/// `channel_name` and `channel_id` are forwarded to `ReminderTool` so it can
/// embed the correct delivery target in the persisted job action.
pub fn build_tools<C: MessageContext + 'static>(
    ctx: Arc<C>,
    channel_name: &str,
    channel_id: Option<u64>,
) -> Vec<Box<dyn Tool>> {
    let mut tools: Vec<Box<dyn Tool>> = vec![
        Box::new(super::read_file::ReadFileTool),
        Box::new(super::write_file::WriteFileTool),
        Box::new(super::list_files::ListFilesTool),
        Box::new(super::search_files::SearchFilesTool),
        Box::new(ExecuteCommandTool::new(Arc::clone(&ctx))),
        Box::new(BashSessionTool::new(Arc::clone(&ctx))),
        Box::new(ReminderTool::new(
            Arc::clone(&ctx),
            channel_name,
            channel_id,
        )),
        Box::new(KnowledgeSearchTool::new(Arc::clone(&ctx))),
        Box::new(KnowledgeWriteTool::new(Arc::clone(&ctx))),
        Box::new(super::patch_file::PatchFileTool),
    ];

    // Load script plugins from ~/.skynet/tools/ — no restart needed after adding a plugin,
    // tools are re-scanned on each build_tools() call (i.e. each new message).
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    let tools_dir = std::path::Path::new(&home).join(".skynet/tools");
    tools.extend(super::script_tool::load_script_tools(&tools_dir));

    tools
}

/// Convert a tool list to API-level definitions for the LLM request.
pub fn tool_definitions(tools: &[Box<dyn Tool>]) -> Vec<ToolDefinition> {
    to_definitions(tools)
}
