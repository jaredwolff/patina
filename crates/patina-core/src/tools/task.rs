use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, OnceLock};

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};

use crate::agent::subagent::SubagentManager;
use crate::persona::PersonaStore;
use crate::session::SessionManager;
use crate::task::{TaskManager, TaskPriority, TaskStatus};
use crate::tools::Tool;

/// Tool for managing Kanban tasks.
pub struct TaskTool {
    manager: Arc<Mutex<TaskManager>>,
    default_channel: Arc<RwLock<String>>,
    default_chat_id: Arc<RwLock<String>>,
    subagent_manager: OnceLock<Arc<SubagentManager>>,
    persona_store: OnceLock<Arc<Mutex<PersonaStore>>>,
    sessions_dir: OnceLock<PathBuf>,
}

impl TaskTool {
    pub fn new(manager: Arc<Mutex<TaskManager>>) -> Self {
        Self {
            manager,
            default_channel: Arc::new(RwLock::new(String::new())),
            default_chat_id: Arc::new(RwLock::new(String::new())),
            subagent_manager: OnceLock::new(),
            persona_store: OnceLock::new(),
            sessions_dir: OnceLock::new(),
        }
    }

    /// Set the subagent manager for auto_execute support. Can only be called once.
    pub fn set_subagent_manager(&self, mgr: Arc<SubagentManager>) {
        let _ = self.subagent_manager.set(mgr);
    }

    /// Set the persona store for resolving assignee personas. Can only be called once.
    pub fn set_persona_store(&self, store: Arc<Mutex<PersonaStore>>) {
        let _ = self.persona_store.set(store);
    }

    /// Set the sessions directory for activity logging. Can only be called once.
    pub fn set_sessions_dir(&self, path: PathBuf) {
        let _ = self.sessions_dir.set(path);
    }

    /// Update the context for task creation attribution.
    pub async fn set_context(&self, channel: &str, chat_id: &str) {
        *self.default_channel.write().await = channel.to_string();
        *self.default_chat_id.write().await = chat_id.to_string();
    }

    async fn session_key(&self) -> String {
        let ch = self.default_channel.read().await;
        let ci = self.default_chat_id.read().await;
        if ch.is_empty() {
            "unknown".to_string()
        } else {
            format!("{ch}:{ci}")
        }
    }

    /// Log an activity event to the task's session timeline.
    async fn log_activity(&self, task_id: &str, event: &str) {
        let sessions_dir = match self.sessions_dir.get() {
            Some(d) => d,
            None => return,
        };

        // Resolve persona name from task's assignee
        let actor = {
            let mgr = self.manager.lock().await;
            if let Some(task) = mgr.get(task_id) {
                if let Some(assignee) = &task.assignee {
                    if let Some(store) = self.persona_store.get() {
                        let ps = store.lock().await;
                        match ps.get(assignee) {
                            Some(p) => format!("Agent ({})", p.name),
                            None => "Agent".to_string(),
                        }
                    } else {
                        "Agent".to_string()
                    }
                } else {
                    "Agent".to_string()
                }
            } else {
                "Agent".to_string()
            }
        };

        let mut sm = SessionManager::new(sessions_dir.clone());
        let key = format!("task:{task_id}");
        let session = sm.get_or_create(&key);
        session.add_message("system", &format!("{actor} {event}"));
        let _ = sm.save(&key);
    }
}

#[async_trait]
impl Tool for TaskTool {
    fn name(&self) -> &str {
        "task"
    }

    fn description(&self) -> &str {
        "Manage Kanban tasks. Create, list, update, move, assign, comment on, or delete tasks.\n\
         Tasks have four status columns: backlog, todo, in_progress, done.\n\
         Tasks can be assigned to a persona for execution.\n\
         Use 'auto_execute: true' with 'assign' to immediately spawn the assigned persona as a subagent to work on the task."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "list", "get", "update", "move", "assign", "comment", "delete"],
                    "description": "The action to perform"
                },
                "title": {
                    "type": "string",
                    "description": "Task title (required for 'add')"
                },
                "description": {
                    "type": "string",
                    "description": "Task description in markdown (for 'add' or 'update')"
                },
                "task_id": {
                    "type": "string",
                    "description": "Task ID (required for get/update/move/assign/comment/delete)"
                },
                "status": {
                    "type": "string",
                    "enum": ["backlog", "todo", "in_progress", "done"],
                    "description": "Target status (required for 'move')"
                },
                "priority": {
                    "type": "string",
                    "enum": ["low", "medium", "high", "urgent"],
                    "description": "Task priority (for 'add' or 'update', default: medium)"
                },
                "assignee": {
                    "type": "string",
                    "description": "Persona key to assign (for 'add' or 'assign')"
                },
                "tags": {
                    "type": "array",
                    "items": { "type": "string" },
                    "description": "Tags (for 'add' or 'update')"
                },
                "comment": {
                    "type": "string",
                    "description": "Comment text (required for 'comment')"
                },
                "filter_status": {
                    "type": "string",
                    "enum": ["backlog", "todo", "in_progress", "done"],
                    "description": "Filter by status (for 'list')"
                },
                "filter_assignee": {
                    "type": "string",
                    "description": "Filter by assignee persona (for 'list')"
                },
                "auto_execute": {
                    "type": "boolean",
                    "description": "If true, spawn assignee persona as subagent to work on the task (for 'assign')"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<String> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: action"))?;

        match action {
            "add" => self.handle_add(&params).await,
            "list" => self.handle_list(&params).await,
            "get" => self.handle_get(&params).await,
            "update" => self.handle_update(&params).await,
            "move" => self.handle_move(&params).await,
            "assign" => self.handle_assign(&params).await,
            "comment" => self.handle_comment(&params).await,
            "delete" => self.handle_delete(&params).await,
            _ => Ok(format!(
                "Unknown action: {action}. Use 'add', 'list', 'get', 'update', 'move', 'assign', 'comment', or 'delete'."
            )),
        }
    }
}

impl TaskTool {
    async fn handle_add(&self, params: &serde_json::Value) -> Result<String> {
        let title = params
            .get("title")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: title"))?;

        let description = params
            .get("description")
            .and_then(|v| v.as_str())
            .unwrap_or("");

        let priority = params
            .get("priority")
            .and_then(|v| v.as_str())
            .and_then(TaskPriority::from_str)
            .unwrap_or(TaskPriority::Medium);

        let assignee = params
            .get("assignee")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let tags: Vec<String> = params
            .get("tags")
            .and_then(|v| v.as_array())
            .map(|arr| {
                arr.iter()
                    .filter_map(|v| v.as_str().map(|s| s.to_string()))
                    .collect()
            })
            .unwrap_or_default();

        let created_by = self.session_key().await;

        let mut mgr = self.manager.lock().await;
        let task = mgr.add(title, description, priority, assignee, tags, &created_by)?;

        Ok(format!(
            "Task '{}' created (ID: {}, status: todo)",
            task.title, task.id
        ))
    }

    async fn handle_list(&self, params: &serde_json::Value) -> Result<String> {
        let filter_status = params
            .get("filter_status")
            .and_then(|v| v.as_str())
            .and_then(TaskStatus::from_str);

        let filter_assignee = params.get("filter_assignee").and_then(|v| v.as_str());

        let mut mgr = self.manager.lock().await;
        let tasks = mgr.list(filter_status.as_ref(), filter_assignee);

        if tasks.is_empty() {
            return Ok("No tasks found.".to_string());
        }

        let mut output = format!("{} task(s):\n", tasks.len());
        for task in tasks {
            let assignee = task.assignee.as_deref().unwrap_or("unassigned");
            let tags_str = if task.tags.is_empty() {
                String::new()
            } else {
                format!(" [{}]", task.tags.join(", "))
            };
            output.push_str(&format!(
                "  [{id}] {status:<11} {priority:<6} {title} — {assignee}{tags}\n",
                id = task.id,
                status = task.status.as_str(),
                priority = format!("{:?}", task.priority).to_lowercase(),
                title = task.title,
                assignee = assignee,
                tags = tags_str,
            ));
        }

        Ok(output)
    }

    async fn handle_get(&self, params: &serde_json::Value) -> Result<String> {
        let task_id = params
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: task_id"))?;

        let mgr = self.manager.lock().await;
        match mgr.get(task_id) {
            Some(task) => {
                let assignee = task.assignee.as_deref().unwrap_or("unassigned");
                let tags = if task.tags.is_empty() {
                    "none".to_string()
                } else {
                    task.tags.join(", ")
                };
                let created = chrono::DateTime::from_timestamp_millis(task.created_at_ms)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
                    .unwrap_or_else(|| "?".to_string());
                let updated = chrono::DateTime::from_timestamp_millis(task.updated_at_ms)
                    .map(|dt| dt.format("%Y-%m-%d %H:%M UTC").to_string())
                    .unwrap_or_else(|| "?".to_string());

                let mut out = format!(
                    "Task {id}\n\
                     Title: {title}\n\
                     Status: {status}\n\
                     Priority: {priority}\n\
                     Assignee: {assignee}\n\
                     Tags: {tags}\n\
                     Created: {created} by {by}\n\
                     Updated: {updated}\n",
                    id = task.id,
                    title = task.title,
                    status = task.status.as_str(),
                    priority = format!("{:?}", task.priority).to_lowercase(),
                    assignee = assignee,
                    tags = tags,
                    created = created,
                    by = task.created_by,
                    updated = updated,
                );

                if !task.description.is_empty() {
                    out.push_str(&format!("Description:\n{}\n", task.description));
                }

                if !task.comments.is_empty() {
                    out.push_str(&format!("\n{} comment(s):\n", task.comments.len()));
                    for c in &task.comments {
                        let ts = chrono::DateTime::from_timestamp_millis(c.timestamp_ms)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_else(|| "?".to_string());
                        out.push_str(&format!("  [{ts}] {}: {}\n", c.author, c.content));
                    }
                }

                Ok(out)
            }
            None => Ok(format!("Task {task_id} not found.")),
        }
    }

    async fn handle_update(&self, params: &serde_json::Value) -> Result<String> {
        let task_id = params
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: task_id"))?;

        let title = params.get("title").and_then(|v| v.as_str());
        let description = params.get("description").and_then(|v| v.as_str());
        let priority = params
            .get("priority")
            .and_then(|v| v.as_str())
            .and_then(TaskPriority::from_str);
        let tags: Option<Vec<String>> = params.get("tags").and_then(|v| v.as_array()).map(|arr| {
            arr.iter()
                .filter_map(|v| v.as_str().map(|s| s.to_string()))
                .collect()
        });

        let mut mgr = self.manager.lock().await;
        if mgr.update(task_id, title, description, priority, tags)? {
            Ok(format!("Task {task_id} updated."))
        } else {
            Ok(format!("Task {task_id} not found."))
        }
    }

    async fn handle_move(&self, params: &serde_json::Value) -> Result<String> {
        let task_id = params
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: task_id"))?;

        let status_str = params
            .get("status")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: status"))?;

        let status = TaskStatus::from_str(status_str)
            .ok_or_else(|| anyhow::anyhow!("invalid status: {status_str}"))?;

        let mut mgr = self.manager.lock().await;
        if mgr.move_task(task_id, status)? {
            drop(mgr);
            self.log_activity(task_id, &format!("changed status to {status_str}"))
                .await;
            Ok(format!("Task {task_id} moved to {status_str}."))
        } else {
            Ok(format!("Task {task_id} not found."))
        }
    }

    async fn handle_assign(&self, params: &serde_json::Value) -> Result<String> {
        let task_id = params
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: task_id"))?;

        let assignee = params.get("assignee").and_then(|v| v.as_str());

        let auto_execute = params
            .get("auto_execute")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        let mut mgr = self.manager.lock().await;
        if !mgr.assign(task_id, assignee)? {
            return Ok(format!("Task {task_id} not found."));
        }

        let assignee_display = assignee.unwrap_or("nobody");

        if auto_execute && assignee.is_some() {
            let assignee_key = assignee.unwrap().to_string();

            // Resolve persona preamble + model tier
            let (preamble, model_tier) = if let Some(store) = self.persona_store.get() {
                let ps = store.lock().await;
                match ps.get(&assignee_key) {
                    Some(persona) => (
                        if persona.preamble.is_empty() {
                            None
                        } else {
                            Some(persona.preamble.clone())
                        },
                        if persona.model_tier.is_empty() {
                            None
                        } else {
                            Some(persona.model_tier.clone())
                        },
                    ),
                    None => (None, None),
                }
            } else {
                (None, None)
            };

            // Build task prompt from title + description
            let task_prompt = {
                let task = mgr.get(task_id).expect("task just assigned");
                if task.description.is_empty() {
                    task.title.clone()
                } else {
                    format!("{}\n\n{}", task.title, task.description)
                }
            };
            let task_title = mgr.get(task_id).unwrap().title.clone();

            // Move to in_progress
            let _ = mgr.move_task(task_id, TaskStatus::InProgress);

            // Drop the TaskManager lock before spawning
            drop(mgr);

            // Spawn subagent
            if let Some(sam) = self.subagent_manager.get() {
                let mut extra = HashMap::new();
                extra.insert("task_id".to_string(), serde_json::json!(task_id));

                match sam.spawn_with_persona(
                    &task_prompt,
                    &task_title,
                    "task",
                    task_id,
                    preamble.as_deref(),
                    model_tier.as_deref(),
                    extra,
                ).await {
                    Ok(subagent_id) => Ok(format!(
                        "Task {task_id} assigned to {assignee_display} and executing (subagent {subagent_id})."
                    )),
                    Err(e) => Ok(format!(
                        "Task {task_id} assigned to {assignee_display} and moved to in_progress, \
                         but failed to spawn subagent: {e}"
                    )),
                }
            } else {
                Ok(format!(
                    "Task {task_id} assigned to {assignee_display} and moved to in_progress. \
                     (auto_execute unavailable — no subagent manager configured)"
                ))
            }
        } else {
            drop(mgr);
            self.log_activity(task_id, &format!("assigned to {assignee_display}"))
                .await;
            Ok(format!("Task {task_id} assigned to {assignee_display}."))
        }
    }

    async fn handle_comment(&self, params: &serde_json::Value) -> Result<String> {
        let task_id = params
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: task_id"))?;

        let comment = params
            .get("comment")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: comment"))?;

        let author = self.session_key().await;

        let mut mgr = self.manager.lock().await;
        if mgr.add_comment(task_id, &author, comment)? {
            Ok(format!("Comment added to task {task_id}."))
        } else {
            Ok(format!("Task {task_id} not found."))
        }
    }

    async fn handle_delete(&self, params: &serde_json::Value) -> Result<String> {
        let task_id = params
            .get("task_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: task_id"))?;

        let mut mgr = self.manager.lock().await;
        if mgr.delete(task_id)? {
            Ok(format!("Task {task_id} deleted."))
        } else {
            Ok(format!("Task {task_id} not found."))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn test_tool() -> (TaskTool, NamedTempFile) {
        let file = NamedTempFile::new().unwrap();
        let mgr = TaskManager::load(file.path());
        let tool = TaskTool::new(Arc::new(Mutex::new(mgr)));
        (tool, file)
    }

    #[tokio::test]
    async fn test_add_action() {
        let (tool, _f) = test_tool();
        let result = tool
            .execute(serde_json::json!({
                "action": "add",
                "title": "Fix bug",
                "description": "Something is broken",
                "priority": "high",
                "tags": ["bug", "urgent"]
            }))
            .await
            .unwrap();
        assert!(result.contains("Fix bug"));
        assert!(result.contains("created"));
    }

    #[tokio::test]
    async fn test_list_action() {
        let (tool, _f) = test_tool();
        tool.execute(serde_json::json!({
            "action": "add",
            "title": "Task A"
        }))
        .await
        .unwrap();
        tool.execute(serde_json::json!({
            "action": "add",
            "title": "Task B"
        }))
        .await
        .unwrap();

        let result = tool
            .execute(serde_json::json!({ "action": "list" }))
            .await
            .unwrap();
        assert!(result.contains("2 task(s)"));
        assert!(result.contains("Task A"));
        assert!(result.contains("Task B"));
    }

    #[tokio::test]
    async fn test_list_empty() {
        let (tool, _f) = test_tool();
        let result = tool
            .execute(serde_json::json!({ "action": "list" }))
            .await
            .unwrap();
        assert!(result.contains("No tasks found"));
    }

    #[tokio::test]
    async fn test_list_filter_status() {
        let (tool, _f) = test_tool();
        // Add a task (starts as todo)
        let result = tool
            .execute(serde_json::json!({
                "action": "add",
                "title": "Filtered task"
            }))
            .await
            .unwrap();
        // Extract task ID
        let id = extract_id(&result);

        // Move to in_progress
        tool.execute(serde_json::json!({
            "action": "move",
            "task_id": id,
            "status": "in_progress"
        }))
        .await
        .unwrap();

        // Filter by todo — should be empty
        let result = tool
            .execute(serde_json::json!({
                "action": "list",
                "filter_status": "todo"
            }))
            .await
            .unwrap();
        assert!(result.contains("No tasks found"));

        // Filter by in_progress — should find it
        let result = tool
            .execute(serde_json::json!({
                "action": "list",
                "filter_status": "in_progress"
            }))
            .await
            .unwrap();
        assert!(result.contains("Filtered task"));
    }

    #[tokio::test]
    async fn test_get_action() {
        let (tool, _f) = test_tool();
        let result = tool
            .execute(serde_json::json!({
                "action": "add",
                "title": "Detailed task",
                "description": "Full details here",
                "priority": "urgent",
                "tags": ["critical"]
            }))
            .await
            .unwrap();
        let id = extract_id(&result);

        let result = tool
            .execute(serde_json::json!({
                "action": "get",
                "task_id": id
            }))
            .await
            .unwrap();
        assert!(result.contains("Detailed task"));
        assert!(result.contains("urgent"));
        assert!(result.contains("Full details here"));
        assert!(result.contains("critical"));
    }

    #[tokio::test]
    async fn test_get_not_found() {
        let (tool, _f) = test_tool();
        let result = tool
            .execute(serde_json::json!({
                "action": "get",
                "task_id": "nonexist"
            }))
            .await
            .unwrap();
        assert!(result.contains("not found"));
    }

    #[tokio::test]
    async fn test_update_action() {
        let (tool, _f) = test_tool();
        let result = tool
            .execute(serde_json::json!({
                "action": "add",
                "title": "Old title"
            }))
            .await
            .unwrap();
        let id = extract_id(&result);

        let result = tool
            .execute(serde_json::json!({
                "action": "update",
                "task_id": id,
                "title": "New title",
                "priority": "low"
            }))
            .await
            .unwrap();
        assert!(result.contains("updated"));

        let result = tool
            .execute(serde_json::json!({
                "action": "get",
                "task_id": id
            }))
            .await
            .unwrap();
        assert!(result.contains("New title"));
        assert!(result.contains("low"));
    }

    #[tokio::test]
    async fn test_move_action() {
        let (tool, _f) = test_tool();
        let result = tool
            .execute(serde_json::json!({
                "action": "add",
                "title": "Moveable"
            }))
            .await
            .unwrap();
        let id = extract_id(&result);

        let result = tool
            .execute(serde_json::json!({
                "action": "move",
                "task_id": id,
                "status": "done"
            }))
            .await
            .unwrap();
        assert!(result.contains("moved to done"));
    }

    #[tokio::test]
    async fn test_assign_action() {
        let (tool, _f) = test_tool();
        let result = tool
            .execute(serde_json::json!({
                "action": "add",
                "title": "Assignable"
            }))
            .await
            .unwrap();
        let id = extract_id(&result);

        let result = tool
            .execute(serde_json::json!({
                "action": "assign",
                "task_id": id,
                "assignee": "coder"
            }))
            .await
            .unwrap();
        assert!(result.contains("assigned to coder"));
    }

    #[tokio::test]
    async fn test_assign_auto_execute() {
        let (tool, _f) = test_tool();
        let result = tool
            .execute(serde_json::json!({
                "action": "add",
                "title": "Auto exec task"
            }))
            .await
            .unwrap();
        let id = extract_id(&result);

        let result = tool
            .execute(serde_json::json!({
                "action": "assign",
                "task_id": id,
                "assignee": "coder",
                "auto_execute": true
            }))
            .await
            .unwrap();
        assert!(result.contains("assigned to coder"));
        assert!(result.contains("in_progress"));

        // Verify the task is actually in_progress
        let result = tool
            .execute(serde_json::json!({
                "action": "get",
                "task_id": id
            }))
            .await
            .unwrap();
        assert!(result.contains("in_progress"));
    }

    #[tokio::test]
    async fn test_comment_action() {
        let (tool, _f) = test_tool();
        let result = tool
            .execute(serde_json::json!({
                "action": "add",
                "title": "Commentable"
            }))
            .await
            .unwrap();
        let id = extract_id(&result);

        let result = tool
            .execute(serde_json::json!({
                "action": "comment",
                "task_id": id,
                "comment": "Looking into this"
            }))
            .await
            .unwrap();
        assert!(result.contains("Comment added"));

        let result = tool
            .execute(serde_json::json!({
                "action": "get",
                "task_id": id
            }))
            .await
            .unwrap();
        assert!(result.contains("Looking into this"));
    }

    #[tokio::test]
    async fn test_delete_action() {
        let (tool, _f) = test_tool();
        let result = tool
            .execute(serde_json::json!({
                "action": "add",
                "title": "Deleteable"
            }))
            .await
            .unwrap();
        let id = extract_id(&result);

        let result = tool
            .execute(serde_json::json!({
                "action": "delete",
                "task_id": id
            }))
            .await
            .unwrap();
        assert!(result.contains("deleted"));

        let result = tool
            .execute(serde_json::json!({
                "action": "get",
                "task_id": id
            }))
            .await
            .unwrap();
        assert!(result.contains("not found"));
    }

    #[tokio::test]
    async fn test_unknown_action() {
        let (tool, _f) = test_tool();
        let result = tool
            .execute(serde_json::json!({ "action": "explode" }))
            .await
            .unwrap();
        assert!(result.contains("Unknown action"));
    }

    #[tokio::test]
    async fn test_missing_action() {
        let (tool, _f) = test_tool();
        let result = tool.execute(serde_json::json!({})).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_context_sets_created_by() {
        let (tool, _f) = test_tool();
        tool.set_context("telegram", "12345").await;

        let result = tool
            .execute(serde_json::json!({
                "action": "add",
                "title": "Contextual"
            }))
            .await
            .unwrap();
        let id = extract_id(&result);

        let result = tool
            .execute(serde_json::json!({
                "action": "get",
                "task_id": id
            }))
            .await
            .unwrap();
        assert!(result.contains("telegram:12345"));
    }

    /// Extract the 8-char task ID from an "add" result like "Task 'Foo' created (ID: abcd1234, status: todo)"
    fn extract_id(result: &str) -> String {
        let start = result.find("ID: ").expect("no ID in result") + 4;
        result[start..start + 8].to_string()
    }
}
