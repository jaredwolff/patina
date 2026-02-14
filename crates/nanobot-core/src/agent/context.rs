use std::path::Path;

use anyhow::Result;

/// Builds the system prompt and message list for LLM calls.
pub struct ContextBuilder {
    workspace: std::path::PathBuf,
}

impl ContextBuilder {
    pub fn new(workspace: &Path) -> Self {
        Self {
            workspace: workspace.to_path_buf(),
        }
    }

    /// Load bootstrap files (AGENTS.md, SOUL.md, USER.md, etc.) into a system prompt.
    pub fn build_system_prompt(&self) -> Result<String> {
        // TODO: load and combine workspace markdown files
        Ok(String::new())
    }
}
