pub mod exec;
pub mod read_file;
pub mod web_fetch;
pub mod web_search;
pub mod write_file;

use closeclaw_core::tool::Tool;
use std::path::Path;
use std::sync::Arc;

/// Create all built-in tools, sandboxed to the given workspace directory.
pub fn builtin_tools(workspace: &Path) -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(exec::ExecTool::new(workspace.to_path_buf())),
        Arc::new(read_file::ReadFileTool::new(workspace.to_path_buf())),
        Arc::new(write_file::WriteFileTool::new(workspace.to_path_buf())),
        Arc::new(web_fetch::WebFetchTool::new()),
        Arc::new(web_search::WebSearchTool::new()),
    ]
}
