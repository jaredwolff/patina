use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use tracing::warn;

/// Kanban column status.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum TaskStatus {
    Backlog,
    Todo,
    InProgress,
    Done,
}

impl TaskStatus {
    pub fn as_str(&self) -> &str {
        match self {
            TaskStatus::Backlog => "backlog",
            TaskStatus::Todo => "todo",
            TaskStatus::InProgress => "in_progress",
            TaskStatus::Done => "done",
        }
    }

    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().replace('-', "_").as_str() {
            "backlog" => Some(Self::Backlog),
            "todo" => Some(Self::Todo),
            "in_progress" | "inprogress" => Some(Self::InProgress),
            "done" => Some(Self::Done),
            _ => None,
        }
    }
}

/// Priority level.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TaskPriority {
    Low,
    Medium,
    High,
    Urgent,
}

impl TaskPriority {
    pub fn from_str(s: &str) -> Option<Self> {
        match s.to_lowercase().as_str() {
            "low" => Some(Self::Low),
            "medium" => Some(Self::Medium),
            "high" => Some(Self::High),
            "urgent" => Some(Self::Urgent),
            _ => None,
        }
    }
}

fn default_priority() -> TaskPriority {
    TaskPriority::Medium
}

/// A comment or activity entry on a task.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskComment {
    pub author: String,
    pub content: String,
    pub timestamp_ms: i64,
}

/// A single task on the Kanban board.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Task {
    pub id: String,
    pub title: String,
    #[serde(default)]
    pub description: String,
    pub status: TaskStatus,
    #[serde(default = "default_priority")]
    pub priority: TaskPriority,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub assignee: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub tags: Vec<String>,
    pub created_by: String,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub completed_at_ms: Option<i64>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub comments: Vec<TaskComment>,
}

/// Top-level persistence structure.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TaskStore {
    pub version: u32,
    pub tasks: Vec<Task>,
}

impl Default for TaskStore {
    fn default() -> Self {
        Self {
            version: 1,
            tasks: Vec::new(),
        }
    }
}

/// Manages task persistence and CRUD operations.
pub struct TaskManager {
    path: PathBuf,
    store: TaskStore,
}

impl TaskManager {
    pub fn load(path: &Path) -> Self {
        let store = if path.exists() {
            match std::fs::read_to_string(path) {
                Ok(content) => serde_json::from_str(&content).unwrap_or_default(),
                Err(e) => {
                    warn!("Failed to read tasks file: {e}");
                    TaskStore::default()
                }
            }
        } else {
            TaskStore::default()
        };
        Self {
            path: path.to_path_buf(),
            store,
        }
    }

    fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&self.store)?;
        std::fs::write(&self.path, json)?;
        Ok(())
    }

    /// Reload from disk before operations (multi-process safety).
    pub fn refresh_from_disk(&mut self) {
        if self.path.exists() {
            match std::fs::read_to_string(&self.path) {
                Ok(content) => {
                    if let Ok(store) = serde_json::from_str(&content) {
                        self.store = store;
                    }
                }
                Err(e) => warn!("Failed to refresh tasks from disk: {e}"),
            }
        }
    }

    pub fn add(
        &mut self,
        title: &str,
        description: &str,
        priority: TaskPriority,
        assignee: Option<String>,
        tags: Vec<String>,
        created_by: &str,
    ) -> Result<Task> {
        self.refresh_from_disk();
        let now = Utc::now().timestamp_millis();
        let id = uuid::Uuid::new_v4().to_string()[..8].to_string();
        let task = Task {
            id: id.clone(),
            title: title.to_string(),
            description: description.to_string(),
            status: TaskStatus::Todo,
            priority,
            assignee,
            tags,
            created_by: created_by.to_string(),
            created_at_ms: now,
            updated_at_ms: now,
            completed_at_ms: None,
            comments: Vec::new(),
        };
        self.store.tasks.push(task.clone());
        self.save()?;
        Ok(task)
    }

    pub fn get(&self, id: &str) -> Option<&Task> {
        self.store.tasks.iter().find(|t| t.id == id)
    }

    pub fn list(
        &mut self,
        filter_status: Option<&TaskStatus>,
        filter_assignee: Option<&str>,
    ) -> Vec<&Task> {
        self.refresh_from_disk();
        self.store
            .tasks
            .iter()
            .filter(|t| {
                if let Some(status) = filter_status {
                    if t.status != *status {
                        return false;
                    }
                }
                if let Some(assignee) = filter_assignee {
                    match &t.assignee {
                        Some(a) if a == assignee => {}
                        _ => return false,
                    }
                }
                true
            })
            .collect()
    }

    pub fn update(
        &mut self,
        id: &str,
        title: Option<&str>,
        description: Option<&str>,
        priority: Option<TaskPriority>,
        tags: Option<Vec<String>>,
    ) -> Result<bool> {
        self.refresh_from_disk();
        let now = Utc::now().timestamp_millis();
        if let Some(task) = self.store.tasks.iter_mut().find(|t| t.id == id) {
            if let Some(t) = title {
                task.title = t.to_string();
            }
            if let Some(d) = description {
                task.description = d.to_string();
            }
            if let Some(p) = priority {
                task.priority = p;
            }
            if let Some(tg) = tags {
                task.tags = tg;
            }
            task.updated_at_ms = now;
            self.save()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn move_task(&mut self, id: &str, status: TaskStatus) -> Result<bool> {
        self.refresh_from_disk();
        let now = Utc::now().timestamp_millis();
        if let Some(task) = self.store.tasks.iter_mut().find(|t| t.id == id) {
            task.status = status.clone();
            task.updated_at_ms = now;
            if status == TaskStatus::Done {
                task.completed_at_ms = Some(now);
            }
            self.save()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn assign(&mut self, id: &str, assignee: Option<&str>) -> Result<bool> {
        self.refresh_from_disk();
        let now = Utc::now().timestamp_millis();
        if let Some(task) = self.store.tasks.iter_mut().find(|t| t.id == id) {
            task.assignee = assignee.map(|s| s.to_string());
            task.updated_at_ms = now;
            self.save()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn add_comment(&mut self, id: &str, author: &str, content: &str) -> Result<bool> {
        self.refresh_from_disk();
        let now = Utc::now().timestamp_millis();
        if let Some(task) = self.store.tasks.iter_mut().find(|t| t.id == id) {
            task.comments.push(TaskComment {
                author: author.to_string(),
                content: content.to_string(),
                timestamp_ms: now,
            });
            task.updated_at_ms = now;
            self.save()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    pub fn delete(&mut self, id: &str) -> Result<bool> {
        self.refresh_from_disk();
        let before = self.store.tasks.len();
        self.store.tasks.retain(|t| t.id != id);
        if self.store.tasks.len() < before {
            self.save()?;
            Ok(true)
        } else {
            Ok(false)
        }
    }

    /// Get all tasks (for API serialization).
    pub fn all_tasks(&mut self) -> &[Task] {
        self.refresh_from_disk();
        &self.store.tasks
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn test_manager() -> (TaskManager, NamedTempFile) {
        let file = NamedTempFile::new().unwrap();
        let manager = TaskManager::load(file.path());
        (manager, file)
    }

    #[test]
    fn test_add_and_get() {
        let (mut mgr, _f) = test_manager();
        let task = mgr
            .add(
                "Test task",
                "Description",
                TaskPriority::High,
                None,
                vec![],
                "test:1",
            )
            .unwrap();
        assert_eq!(task.title, "Test task");
        assert_eq!(task.status, TaskStatus::Todo);
        assert_eq!(task.priority, TaskPriority::High);
        assert!(mgr.get(&task.id).is_some());
    }

    #[test]
    fn test_list_filter_status() {
        let (mut mgr, _f) = test_manager();
        let t1 = mgr
            .add("Task 1", "", TaskPriority::Medium, None, vec![], "test:1")
            .unwrap();
        mgr.add("Task 2", "", TaskPriority::Medium, None, vec![], "test:1")
            .unwrap();
        mgr.move_task(&t1.id, TaskStatus::Done).unwrap();

        let done = mgr.list(Some(&TaskStatus::Done), None);
        assert_eq!(done.len(), 1);
        assert_eq!(done[0].title, "Task 1");

        let todo = mgr.list(Some(&TaskStatus::Todo), None);
        assert_eq!(todo.len(), 1);
        assert_eq!(todo[0].title, "Task 2");
    }

    #[test]
    fn test_list_filter_assignee() {
        let (mut mgr, _f) = test_manager();
        mgr.add(
            "Task A",
            "",
            TaskPriority::Medium,
            Some("coder".to_string()),
            vec![],
            "test:1",
        )
        .unwrap();
        mgr.add("Task B", "", TaskPriority::Medium, None, vec![], "test:1")
            .unwrap();

        let assigned = mgr.list(None, Some("coder"));
        assert_eq!(assigned.len(), 1);
        assert_eq!(assigned[0].title, "Task A");
    }

    #[test]
    fn test_update() {
        let (mut mgr, _f) = test_manager();
        let task = mgr
            .add("Old title", "", TaskPriority::Low, None, vec![], "test:1")
            .unwrap();
        let updated = mgr
            .update(
                &task.id,
                Some("New title"),
                None,
                Some(TaskPriority::Urgent),
                None,
            )
            .unwrap();
        assert!(updated);
        let t = mgr.get(&task.id).unwrap();
        assert_eq!(t.title, "New title");
        assert_eq!(t.priority, TaskPriority::Urgent);
    }

    #[test]
    fn test_move_task() {
        let (mut mgr, _f) = test_manager();
        let task = mgr
            .add("Task", "", TaskPriority::Medium, None, vec![], "test:1")
            .unwrap();
        mgr.move_task(&task.id, TaskStatus::InProgress).unwrap();
        assert_eq!(mgr.get(&task.id).unwrap().status, TaskStatus::InProgress);
        assert!(mgr.get(&task.id).unwrap().completed_at_ms.is_none());

        mgr.move_task(&task.id, TaskStatus::Done).unwrap();
        assert_eq!(mgr.get(&task.id).unwrap().status, TaskStatus::Done);
        assert!(mgr.get(&task.id).unwrap().completed_at_ms.is_some());
    }

    #[test]
    fn test_assign() {
        let (mut mgr, _f) = test_manager();
        let task = mgr
            .add("Task", "", TaskPriority::Medium, None, vec![], "test:1")
            .unwrap();
        mgr.assign(&task.id, Some("coder")).unwrap();
        assert_eq!(
            mgr.get(&task.id).unwrap().assignee,
            Some("coder".to_string())
        );
        mgr.assign(&task.id, None).unwrap();
        assert_eq!(mgr.get(&task.id).unwrap().assignee, None);
    }

    #[test]
    fn test_add_comment() {
        let (mut mgr, _f) = test_manager();
        let task = mgr
            .add("Task", "", TaskPriority::Medium, None, vec![], "test:1")
            .unwrap();
        mgr.add_comment(&task.id, "user", "This is a note").unwrap();
        let t = mgr.get(&task.id).unwrap();
        assert_eq!(t.comments.len(), 1);
        assert_eq!(t.comments[0].content, "This is a note");
    }

    #[test]
    fn test_delete() {
        let (mut mgr, _f) = test_manager();
        let task = mgr
            .add("Task", "", TaskPriority::Medium, None, vec![], "test:1")
            .unwrap();
        assert!(mgr.delete(&task.id).unwrap());
        assert!(mgr.get(&task.id).is_none());
        assert!(!mgr.delete("nonexistent").unwrap());
    }

    #[test]
    fn test_persistence_roundtrip() {
        let file = NamedTempFile::new().unwrap();
        let path = file.path().to_path_buf();

        {
            let mut mgr = TaskManager::load(&path);
            mgr.add(
                "Persisted",
                "desc",
                TaskPriority::High,
                None,
                vec!["rust".to_string()],
                "test:1",
            )
            .unwrap();
        }

        let mgr = TaskManager::load(&path);
        assert_eq!(mgr.store.tasks.len(), 1);
        assert_eq!(mgr.store.tasks[0].title, "Persisted");
        assert_eq!(mgr.store.tasks[0].tags, vec!["rust"]);
    }

    #[test]
    fn test_delete_nonexistent() {
        let (mut mgr, _f) = test_manager();
        assert!(!mgr.delete("nope").unwrap());
    }

    #[test]
    fn test_update_nonexistent() {
        let (mut mgr, _f) = test_manager();
        assert!(!mgr.update("nope", Some("x"), None, None, None).unwrap());
    }

    #[test]
    fn test_status_from_str() {
        assert_eq!(
            TaskStatus::from_str("in_progress"),
            Some(TaskStatus::InProgress)
        );
        assert_eq!(
            TaskStatus::from_str("InProgress"),
            Some(TaskStatus::InProgress)
        );
        assert_eq!(
            TaskStatus::from_str("in-progress"),
            Some(TaskStatus::InProgress)
        );
        assert_eq!(TaskStatus::from_str("todo"), Some(TaskStatus::Todo));
        assert_eq!(TaskStatus::from_str("invalid"), None);
    }
}
