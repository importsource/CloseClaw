pub mod exec;
pub mod read_file;
pub mod sandbox;
pub mod schedule;
pub mod web_fetch;
pub mod web_search;
pub mod write_file;

use closeclaw_core::schedule::ScheduleHandle;
use closeclaw_core::tool::Tool;
use std::path::Path;
use std::sync::Arc;

/// Create all built-in tools rooted at the given workspace directory.
pub fn builtin_tools(workspace: &Path) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(exec::ExecTool::new(workspace.to_path_buf())),
        Arc::new(read_file::ReadFileTool::new(workspace.to_path_buf())),
        Arc::new(write_file::WriteFileTool::new(workspace.to_path_buf())),
        Arc::new(web_fetch::WebFetchTool::new()),
        Arc::new(web_search::WebSearchTool::new()),
        Arc::new(sandbox::ListFilesTool::new(workspace.to_path_buf())),
        Arc::new(sandbox::CreateFileTool::new(workspace.to_path_buf())),
        Arc::new(sandbox::DeleteFileTool::new(workspace.to_path_buf())),
        Arc::new(sandbox::SearchFilesTool::new(workspace.to_path_buf())),
    ]
}

/// Create schedule management tools backed by the given handle.
pub fn schedule_tools(handle: Arc<dyn ScheduleHandle>) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(schedule::AddScheduleTool::new(handle.clone())),
        Arc::new(schedule::RemoveScheduleTool::new(handle.clone())),
        Arc::new(schedule::ListSchedulesTool::new(handle)),
    ]
}
