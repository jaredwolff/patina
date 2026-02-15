use std::path::{Path, PathBuf};

use anyhow::Result;
use base64::{engine::general_purpose, Engine as _};

use crate::agent::memory::MemoryStore;
use crate::agent::skills::SkillsLoader;
use crate::session::Message;

/// Bootstrap files loaded into the system prompt.
const BOOTSTRAP_FILES: &[&str] = &["AGENTS.md", "SOUL.md", "USER.md", "TOOLS.md", "IDENTITY.md"];

/// Builds the system prompt and message list for LLM calls.
pub struct ContextBuilder {
    workspace: PathBuf,
    memory: MemoryStore,
    skills: SkillsLoader,
    /// Optional override for the system prompt (used by subagents).
    preamble_override: Option<String>,
}

impl ContextBuilder {
    pub fn new(workspace: &Path, builtin_skills: Option<&Path>) -> Self {
        Self {
            workspace: workspace.to_path_buf(),
            memory: MemoryStore::new(workspace),
            skills: SkillsLoader::new(workspace, builtin_skills),
            preamble_override: None,
        }
    }

    /// Create a ContextBuilder with a custom preamble (for subagents).
    pub fn with_preamble(workspace: &Path, preamble: String) -> Self {
        Self {
            workspace: workspace.to_path_buf(),
            memory: MemoryStore::new(workspace),
            skills: SkillsLoader::new(workspace, None),
            preamble_override: Some(preamble),
        }
    }

    /// Access the memory store for consolidation.
    pub fn memory(&self) -> &MemoryStore {
        &self.memory
    }

    /// Build the full system prompt from identity, bootstrap files, skills, and memory.
    pub fn build_system_prompt(&self) -> Result<String> {
        // If a preamble override is set, use it directly (for subagents)
        if let Some(ref preamble) = self.preamble_override {
            return Ok(preamble.clone());
        }

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

        // Always-loaded skills (full content)
        let always_skills = self.skills.get_always_skills();
        if !always_skills.is_empty() {
            let always_content = self.skills.load_skills_for_context(&always_skills);
            if !always_content.is_empty() {
                parts.push(format!("# Active Skills\n\n{always_content}"));
            }
        }

        // Skills summary (progressive loading — agent uses read_file to load full content)
        let skills_summary = self.skills.build_skills_summary();
        if !skills_summary.is_empty() {
            parts.push(format!(
                "# Skills\n\n\
                 The following skills extend your capabilities. To use a skill, \
                 read its SKILL.md file using the read_file tool.\n\
                 Skills with available=\"false\" need dependencies installed first.\n\n\
                 {skills_summary}"
            ));
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
            r#"# Patina

You are Patina, a helpful AI assistant. You have access to tools that allow you to:
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

    /// Build user message content with optional base64-encoded images.
    fn build_user_content(&self, text: &str, media: Option<&[String]>) -> serde_json::Value {
        let media = match media {
            Some(m) if !m.is_empty() => m,
            _ => return serde_json::json!(text),
        };

        let mut content_parts: Vec<serde_json::Value> = Vec::new();

        for path_str in media {
            let path = Path::new(path_str);
            if !path.is_file() {
                continue;
            }
            let mime = mime_guess::from_path(path)
                .first()
                .map(|m| m.to_string())
                .unwrap_or_default();
            if !mime.starts_with("image/") {
                continue;
            }
            if let Ok(bytes) = std::fs::read(path) {
                let b64 = general_purpose::STANDARD.encode(&bytes);
                content_parts.push(serde_json::json!({
                    "type": "image_url",
                    "image_url": {
                        "url": format!("data:{mime};base64,{b64}")
                    }
                }));
            }
        }

        if content_parts.is_empty() {
            return serde_json::json!(text);
        }

        content_parts.push(serde_json::json!({
            "type": "text",
            "text": text
        }));

        serde_json::json!(content_parts)
    }

    /// Build the complete message list for an LLM call.
    pub fn build_messages(
        &self,
        history: &[&Message],
        current_message: &str,
        channel: Option<&str>,
        chat_id: Option<&str>,
        media: Option<&[String]>,
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
            let mut entry = serde_json::json!({
                "role": msg.role,
                "content": msg.content
            });
            // Include reasoning_content for thinking models (Kimi, DeepSeek-R1) so
            // they can see their own reasoning in history.
            if let Some(ref reasoning) = msg.reasoning_content {
                entry["reasoning_content"] = serde_json::Value::String(reasoning.clone());
            }
            messages.push(entry);
        }

        // Current message (with optional media)
        let user_content = self.build_user_content(current_message, media);
        messages.push(serde_json::json!({
            "role": "user",
            "content": user_content
        }));

        Ok(messages)
    }
}
