use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::RwLock;

use crate::agent::subagent::SubagentManager;
use crate::tools::Tool;

/// Tool for spawning background subagent tasks.
pub struct SpawnTool {
    manager: Arc<SubagentManager>,
    default_channel: Arc<RwLock<String>>,
    default_chat_id: Arc<RwLock<String>>,
}

impl SpawnTool {
    pub fn new(manager: Arc<SubagentManager>) -> Self {
        Self {
            manager,
            default_channel: Arc::new(RwLock::new(String::new())),
            default_chat_id: Arc::new(RwLock::new(String::new())),
        }
    }

    /// Update the origin context so subagent results route back correctly.
    pub async fn set_context(&self, channel: &str, chat_id: &str) {
        *self.default_channel.write().await = channel.to_string();
        *self.default_chat_id.write().await = chat_id.to_string();
    }
}

#[async_trait]
impl Tool for SpawnTool {
    fn name(&self) -> &str {
        "spawn"
    }

    fn description(&self) -> &str {
        "Spawn a background subagent to work on a task independently. The subagent runs \
         in the background with its own tool set (file, shell, web) and reports back when done. \
         Use this for tasks that can run concurrently, like research, file processing, or \
         code generation that doesn't need your immediate attention."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "task": {
                    "type": "string",
                    "description": "Detailed description of the task for the subagent to perform"
                },
                "label": {
                    "type": "string",
                    "description": "Short label for identifying this subagent (e.g. 'research-api', 'fix-tests')"
                }
            },
            "required": ["task"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<String> {
        let task = params
            .get("task")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: task"))?;

        let label = params.get("label").and_then(|v| v.as_str()).unwrap_or("");

        let channel = self.default_channel.read().await.clone();
        let chat_id = self.default_chat_id.read().await.clone();

        if channel.is_empty() || chat_id.is_empty() {
            return Ok("Error: No context set for subagent result delivery. Cannot spawn.".into());
        }

        match self.manager.spawn(task, label, &channel, &chat_id).await {
            Ok(task_id) => {
                let label_display = if label.is_empty() {
                    format!("subagent-{task_id}")
                } else {
                    label.to_string()
                };
                Ok(format!(
                    "Subagent '{label_display}' spawned (ID: {task_id}). \
                     It will work on the task in the background and report back when done."
                ))
            }
            Err(e) => Ok(format!("Failed to spawn subagent: {e}")),
        }
    }
}
