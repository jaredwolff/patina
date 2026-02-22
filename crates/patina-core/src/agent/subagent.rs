use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use tokio::sync::{mpsc, Mutex};
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::agent::context::ContextBuilder;
use crate::agent::model_pool::ModelPool;
use crate::agent::r#loop::AgentLoop;
use crate::bus::InboundMessage;
use crate::session::SessionManager;
use crate::tools::filesystem::{EditFileTool, ListDirTool, ReadFileTool, WriteFileTool};
use crate::tools::shell::ExecTool;
use crate::tools::web::{WebFetchTool, WebSearchTool};
use crate::tools::ToolRegistry;
use crate::usage::UsageTracker;

/// Info about a running subagent.
struct SubagentInfo {
    label: String,
    handle: JoinHandle<()>,
}

/// Manages spawning of background agent instances.
pub struct SubagentManager {
    running: Arc<Mutex<HashMap<String, SubagentInfo>>>,
    models: ModelPool,
    workspace: PathBuf,
    inbound_tx: mpsc::Sender<InboundMessage>,
    config: patina_config::Config,
    usage_tracker: Option<Arc<UsageTracker>>,
}

impl SubagentManager {
    pub fn new(
        models: ModelPool,
        workspace: PathBuf,
        inbound_tx: mpsc::Sender<InboundMessage>,
        config: patina_config::Config,
    ) -> Self {
        Self {
            running: Arc::new(Mutex::new(HashMap::new())),
            models,
            workspace,
            inbound_tx,
            config,
            usage_tracker: None,
        }
    }

    /// Set the usage tracker for subagent LLM calls.
    pub fn set_usage_tracker(&mut self, tracker: Arc<UsageTracker>) {
        self.usage_tracker = Some(tracker);
    }

    /// Spawn a background agent task.
    pub async fn spawn(
        &self,
        task: &str,
        label: &str,
        origin_channel: &str,
        origin_chat_id: &str,
    ) -> Result<String> {
        self.spawn_with_persona(
            task,
            label,
            origin_channel,
            origin_chat_id,
            None,
            None,
            HashMap::new(),
        )
        .await
    }

    /// Spawn a background agent with optional persona preamble and model tier.
    ///
    /// When `preamble` is provided, the subagent uses that persona's system prompt
    /// instead of the generic worker prompt. When `model_tier` is provided, the
    /// subagent's LLM calls use that tier. Extra metadata (e.g. `task_id`) is
    /// forwarded in the completion message for downstream handling.
    pub async fn spawn_with_persona(
        &self,
        task: &str,
        label: &str,
        origin_channel: &str,
        origin_chat_id: &str,
        preamble: Option<&str>,
        model_tier: Option<&str>,
        extra_metadata: HashMap<String, serde_json::Value>,
    ) -> Result<String> {
        let task_id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let label_str = if label.is_empty() {
            format!("subagent-{task_id}")
        } else {
            label.to_string()
        };

        info!("Spawning subagent {task_id} ({label_str}): {task}");

        // Build isolated agent loop with persona-aware prompt
        let agent_loop = self.build_subagent_loop_with_persona(&task_id, preamble)?;

        let task_owned = task.to_string();
        let label_owned = label_str.clone();
        let task_id_owned = task_id.clone();
        let origin_channel = origin_channel.to_string();
        let origin_chat_id = origin_chat_id.to_string();
        let inbound_tx = self.inbound_tx.clone();
        let running = self.running.clone();
        let preamble_owned = preamble.map(|s| s.to_string());
        let model_tier_owned = model_tier.map(|s| s.to_string());

        let handle = tokio::spawn(async move {
            // Task-origin subagents write to the task session (unified timeline).
            // All others get their own isolated session.
            let session_key = if origin_channel == "task" {
                format!("task:{origin_chat_id}")
            } else {
                format!("subagent:{task_id_owned}")
            };

            let result = Self::run_subagent_with_persona(
                agent_loop,
                &session_key,
                &task_owned,
                preamble_owned.as_deref(),
                model_tier_owned.as_deref(),
            )
            .await;

            // Format result announcement
            let announcement = match &result {
                Ok(response) => {
                    format!(
                        "[Subagent '{label_owned}' completed]\n\
                         Task: {task_owned}\n\
                         Result: {response}"
                    )
                }
                Err(e) => {
                    format!(
                        "[Subagent '{label_owned}' failed]\n\
                         Task: {task_owned}\n\
                         Error: {e}"
                    )
                }
            };

            // Send result back through the message bus
            let msg = InboundMessage {
                channel: "system".to_string(),
                sender_id: "subagent".to_string(),
                chat_id: format!("{origin_channel}:{origin_chat_id}"),
                content: announcement,
                media: Vec::new(),
                timestamp: crate::bus::default_timestamp(),
                metadata: {
                    let mut m = extra_metadata;
                    m.insert(
                        "subagent_id".to_string(),
                        serde_json::Value::String(task_id_owned.clone()),
                    );
                    m.insert(
                        "status".to_string(),
                        serde_json::Value::String(
                            if result.is_ok() { "completed" } else { "error" }.to_string(),
                        ),
                    );
                    m
                },
            };

            if let Err(e) = inbound_tx.send(msg).await {
                warn!("Failed to announce subagent result: {e}");
            }

            // Cleanup
            running.lock().await.remove(&task_id_owned);
            info!("Subagent {task_id_owned} finished");
        });

        self.running.lock().await.insert(
            task_id.clone(),
            SubagentInfo {
                label: label_str.clone(),
                handle,
            },
        );

        Ok(task_id)
    }

    /// List running subagents.
    pub async fn list(&self) -> Vec<(String, String)> {
        self.running
            .lock()
            .await
            .iter()
            .map(|(id, info)| (id.clone(), info.label.clone()))
            .collect()
    }

    /// Cancel a running subagent.
    pub async fn cancel(&self, task_id: &str) -> bool {
        if let Some(info) = self.running.lock().await.remove(task_id) {
            info.handle.abort();
            info!("Cancelled subagent {task_id}");
            true
        } else {
            false
        }
    }

    fn build_subagent_loop_with_persona(
        &self,
        _task_id: &str,
        preamble: Option<&str>,
    ) -> Result<AgentLoop> {
        let sessions_dir = dirs::home_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join(".patina")
            .join("sessions");
        let sessions = SessionManager::new(sessions_dir);

        // If a persona preamble is provided, use it with task-focus rules appended.
        // Otherwise fall back to the generic worker prompt.
        let subagent_prompt = if let Some(persona_preamble) = preamble {
            format!(
                "{persona_preamble}\n\n\
                 You are working as a background agent on a specific task.\n\
                 Your workspace is: {}\n\
                 Stay focused ONLY on your assigned task.\n\
                 When done, provide a clear summary of what you accomplished.",
                self.workspace.display()
            )
        } else {
            format!(
                "You are a focused background worker agent (subagent). \
                 Your workspace is: {}\n\n\
                 IMPORTANT RULES:\n\
                 - Stay focused ONLY on your assigned task\n\
                 - Do NOT start conversations or ask questions\n\
                 - Do NOT work on anything besides your task\n\
                 - Be concise but thorough in your work\n\
                 - When done, provide a clear summary of what you accomplished",
                self.workspace.display()
            )
        };

        let context = ContextBuilder::with_preamble(&self.workspace, subagent_prompt);

        // Restricted tool set — no message, spawn, or cron tools
        let mut tools = ToolRegistry::new();
        let allowed_dir: Option<PathBuf> = if self.config.tools.restrict_to_workspace {
            Some(self.workspace.clone())
        } else {
            None
        };
        tools.register(Box::new(ReadFileTool::new(allowed_dir.clone())));
        tools.register(Box::new(WriteFileTool::new(allowed_dir.clone())));
        tools.register(Box::new(EditFileTool::new(allowed_dir.clone())));
        tools.register(Box::new(ListDirTool::new(allowed_dir)));
        tools.register(Box::new(ExecTool::new(
            self.workspace.clone(),
            self.config.tools.exec.timeout_secs,
            self.config.tools.restrict_to_workspace,
        )));

        let brave_api_key = if self.config.tools.web.search.api_key.is_empty() {
            std::env::var("BRAVE_API_KEY").unwrap_or_default()
        } else {
            self.config.tools.web.search.api_key.clone()
        };
        tools.register(Box::new(WebSearchTool::new(
            brave_api_key,
            self.config.tools.web.search.max_results,
        )));
        tools.register(Box::new(WebFetchTool::new(50_000)));

        Ok(AgentLoop {
            models: self.models.clone(),
            sessions,
            context,
            tools,
            max_iterations: 15, // Lower limit for subagents
            temperature: self.config.agents.defaults.temperature as f64,
            max_tokens: self.config.agents.defaults.max_tokens as u64,
            memory_window: self.config.agents.defaults.memory_window,
            model_overrides: crate::agent::r#loop::ModelOverrides::defaults(),
            memory_index: None,
            channel_rules: std::collections::HashMap::new(),
            usage_tracker: self.usage_tracker.clone(),
            stream_tx: None,
        })
    }

    async fn run_subagent_with_persona(
        mut agent_loop: AgentLoop,
        session_key: &str,
        task: &str,
        preamble_override: Option<&str>,
        model_tier: Option<&str>,
    ) -> Result<String> {
        let (response, _) = agent_loop
            .process_message_with_persona(session_key, task, None, preamble_override, model_tier)
            .await?;
        Ok(response)
    }
}

impl Default for SubagentManager {
    fn default() -> Self {
        panic!("SubagentManager requires explicit construction with model and config")
    }
}
