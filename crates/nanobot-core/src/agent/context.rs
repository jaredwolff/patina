use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::agent::memory::MemoryStore;
use crate::session::Message;

/// Bootstrap files loaded into the system prompt.
const BOOTSTRAP_FILES: &[&str] = &["AGENTS.md", "SOUL.md", "USER.md", "TOOLS.md", "IDENTITY.md"];

/// Builds the system prompt and message list for LLM calls.
pub struct ContextBuilder {
    workspace: PathBuf,
    memory: MemoryStore,
}

impl ContextBuilder {
    pub fn new(workspace: &Path) -> Self {
        Self {
            workspace: workspace.to_path_buf(),
            memory: MemoryStore::new(workspace),
        }
    }

    /// Build the full system prompt from identity, bootstrap files, and memory.
    pub fn build_system_prompt(&self) -> Result<String> {
        let mut parts = Vec::new();

        // Core identity
        parts.push(self.get_identity());

        // Bootstrap files
        let bootstrap = self.load_bootstrap_files();
        if !bootstrap.is_empty() {
            parts.push(bootstrap);
        }

        // Memory context
        let memory = self.memory.read_long_term().unwrap_or_default();
        if !memory.is_empty() {
            parts.push(format!("# Memory\n\n{memory}"));
        }

        Ok(parts.join("\n\n---\n\n"))
    }

    fn get_identity(&self) -> String {
        let now = chrono::Local::now().format("%Y-%m-%d %H:%M (%A)");
        let tz = chrono::Local::now().format("%Z");
        let workspace_path = self
            .workspace
            .canonicalize()
            .unwrap_or_else(|_| self.workspace.clone())
            .display()
            .to_string();
        let os = std::env::consts::OS;
        let arch = std::env::consts::ARCH;

        format!(
            r#"# nanobot

You are nanobot, a helpful AI assistant. You have access to tools that allow you to:
- Read, write, and edit files
- Execute shell commands
- Search the web and fetch web pages
- Send messages to users on chat channels

## Current Time
{now} ({tz})

## Runtime
{os} {arch}, Rust

## Workspace
Your workspace is at: {workspace_path}
- Long-term memory: {workspace_path}/memory/MEMORY.md
- History log: {workspace_path}/memory/HISTORY.md (grep-searchable)
- Custom skills: {workspace_path}/skills/{{skill-name}}/SKILL.md

IMPORTANT: When responding to direct questions or conversations, reply directly with your text response.
Only use the 'message' tool when you need to send a message to a specific chat channel.
For normal conversation, just respond with text - do not call the message tool.

Always be helpful, accurate, and concise. When using tools, think step by step.
When remembering something important, write to {workspace_path}/memory/MEMORY.md
To recall past events, grep {workspace_path}/memory/HISTORY.md"#
        )
    }

    fn load_bootstrap_files(&self) -> String {
        let mut parts = Vec::new();

        for filename in BOOTSTRAP_FILES {
            let file_path = self.workspace.join(filename);
            if file_path.exists() {
                if let Ok(content) = std::fs::read_to_string(&file_path) {
                    parts.push(format!("## {filename}\n\n{content}"));
                }
            }
        }

        parts.join("\n\n")
    }

    /// Build the complete message list for an LLM call.
    pub fn build_messages(
        &self,
        history: &[&Message],
        current_message: &str,
        channel: Option<&str>,
        chat_id: Option<&str>,
    ) -> Result<Vec<serde_json::Value>> {
        let mut messages = Vec::new();

        // System prompt
        let mut system_prompt = self.build_system_prompt()?;
        if let (Some(ch), Some(cid)) = (channel, chat_id) {
            system_prompt.push_str(&format!(
                "\n\n## Current Session\nChannel: {ch}\nChat ID: {cid}"
            ));
        }
        messages.push(serde_json::json!({
            "role": "system",
            "content": system_prompt
        }));

        // History
        for msg in history {
            messages.push(serde_json::json!({
                "role": msg.role,
                "content": msg.content
            }));
        }

        // Current message
        messages.push(serde_json::json!({
            "role": "user",
            "content": current_message
        }));

        Ok(messages)
    }
}
