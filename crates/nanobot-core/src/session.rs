use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::{Context, Result};
use chrono::{DateTime, Local, Utc};
use serde::{Deserialize, Serialize};

/// A single message in a session.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Message {
    pub role: String,
    pub content: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub timestamp: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tools_used: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reasoning_content: Option<String>,
}

/// JSONL metadata line (first line of session file).
#[derive(Debug, Clone, Serialize, Deserialize)]
struct SessionMetadata {
    #[serde(rename = "_type")]
    type_field: String,
    created_at: String,
    updated_at: String,
    #[serde(default)]
    metadata: HashMap<String, serde_json::Value>,
    #[serde(default)]
    last_consolidated: usize,
}

/// A conversation session.
pub struct Session {
    pub key: String,
    pub messages: Vec<Message>,
    pub created_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub metadata: HashMap<String, serde_json::Value>,
    pub last_consolidated: usize,
}

impl Session {
    pub fn new(key: String) -> Self {
        let now = Utc::now();
        Self {
            key,
            messages: Vec::new(),
            created_at: now,
            updated_at: now,
            metadata: HashMap::new(),
            last_consolidated: 0,
        }
    }

    pub fn add_message(&mut self, role: &str, content: &str) {
        self.messages.push(Message {
            role: role.into(),
            content: content.into(),
            timestamp: Some(Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()),
            tools_used: None,
            reasoning_content: None,
        });
        self.updated_at = Utc::now();
    }

    pub fn add_message_with_tools(&mut self, role: &str, content: &str, tools: Vec<String>) {
        self.messages.push(Message {
            role: role.into(),
            content: content.into(),
            timestamp: Some(Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()),
            tools_used: if tools.is_empty() { None } else { Some(tools) },
            reasoning_content: None,
        });
        self.updated_at = Utc::now();
    }

    /// Add a message with tools and optional reasoning content from thinking models.
    pub fn add_message_full(
        &mut self,
        role: &str,
        content: &str,
        tools: Vec<String>,
        reasoning: Option<String>,
    ) {
        self.messages.push(Message {
            role: role.into(),
            content: content.into(),
            timestamp: Some(Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()),
            tools_used: if tools.is_empty() { None } else { Some(tools) },
            reasoning_content: reasoning,
        });
        self.updated_at = Utc::now();
    }

    /// Get recent messages (role + content only) for LLM history.
    pub fn get_history(&self, max_messages: usize) -> Vec<&Message> {
        let start = self.messages.len().saturating_sub(max_messages);
        self.messages[start..].iter().collect()
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.last_consolidated = 0;
        self.updated_at = Utc::now();
    }
}

/// Manages multiple sessions with JSONL persistence.
/// Compatible with Python nanobot's session format.
pub struct SessionManager {
    sessions_dir: PathBuf,
    pub sessions: HashMap<String, Session>,
}

impl SessionManager {
    pub fn new(sessions_dir: PathBuf) -> Self {
        if let Err(e) = std::fs::create_dir_all(&sessions_dir) {
            tracing::warn!(
                "Failed to create sessions directory '{}': {e}",
                sessions_dir.display()
            );
        }
        Self {
            sessions_dir,
            sessions: HashMap::new(),
        }
    }

    /// Get the file path for a session key.
    fn session_path(&self, key: &str) -> PathBuf {
        // Replace : with _ for filesystem safety (matches Python's safe_filename)
        let safe_key = key.replace(':', "_");
        self.sessions_dir.join(format!("{safe_key}.jsonl"))
    }

    /// Get or create a session, loading from disk if it exists.
    pub fn get_or_create(&mut self, key: &str) -> &mut Session {
        if !self.sessions.contains_key(key) {
            let session = match self.load(key) {
                Ok(Some(s)) => s,
                Ok(None) => Session::new(key.into()),
                Err(e) => {
                    tracing::error!("Failed to load session '{key}': {e}. Creating a new session.");
                    Session::new(key.into())
                }
            };
            self.sessions.insert(key.into(), session);
        }
        self.sessions
            .get_mut(key)
            .expect("session inserted but missing from cache")
    }

    /// Get or create a session, returning I/O errors when load fails.
    pub fn get_or_create_checked(&mut self, key: &str) -> Result<&mut Session> {
        if !self.sessions.contains_key(key) {
            let session = self.load(key)?.unwrap_or_else(|| Session::new(key.into()));
            self.sessions.insert(key.into(), session);
        }
        self.sessions
            .get_mut(key)
            .context("session inserted but missing from cache")
    }

    /// Load a session from its JSONL file.
    fn load(&self, key: &str) -> Result<Option<Session>> {
        let path = self.session_path(key);
        if !path.exists() {
            return Ok(None);
        }

        let file = std::fs::File::open(&path)
            .with_context(|| format!("failed to open session file '{}'", path.display()))?;
        let reader = std::io::BufReader::new(file);

        let mut messages = Vec::new();
        let mut metadata = HashMap::new();
        let mut created_at = Utc::now();
        let mut last_consolidated = 0;

        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => continue,
            };
            let line = line.trim().to_string();
            if line.is_empty() {
                continue;
            }

            let data: serde_json::Value = match serde_json::from_str(&line) {
                Ok(d) => d,
                Err(e) => {
                    tracing::warn!("Skipping malformed JSONL line: {e}");
                    continue;
                }
            };

            if data.get("_type").and_then(|v| v.as_str()) == Some("metadata") {
                // Parse metadata line
                if let Some(ca) = data.get("created_at").and_then(|v| v.as_str()) {
                    created_at = DateTime::parse_from_rfc3339(ca)
                        .map(|d| d.with_timezone(&Utc))
                        .or_else(|_| {
                            // Python uses isoformat() which may not have timezone
                            chrono::NaiveDateTime::parse_from_str(ca, "%Y-%m-%dT%H:%M:%S%.f")
                                .map(|d| d.and_utc())
                        })
                        .unwrap_or_else(|_| Utc::now());
                }
                if let Some(m) = data.get("metadata").and_then(|v| v.as_object()) {
                    metadata = m.iter().map(|(k, v)| (k.clone(), v.clone())).collect();
                }
                if let Some(lc) = data.get("last_consolidated").and_then(|v| v.as_u64()) {
                    last_consolidated = lc as usize;
                }
            } else {
                // Parse message line
                if let Ok(msg) = serde_json::from_value::<Message>(data) {
                    messages.push(msg);
                }
            }
        }

        Ok(Some(Session {
            key: key.into(),
            messages,
            created_at,
            updated_at: Utc::now(),
            metadata,
            last_consolidated,
        }))
    }

    /// Save a session to its JSONL file.
    pub fn save(&self, key: &str) -> Result<()> {
        let session = self
            .sessions
            .get(key)
            .ok_or_else(|| anyhow::anyhow!("session not in cache: {key}"))?;

        let path = self.session_path(key);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let mut file = std::fs::File::create(&path)?;

        // Write metadata line first
        let meta = SessionMetadata {
            type_field: "metadata".into(),
            created_at: session.created_at.to_rfc3339(),
            updated_at: session.updated_at.to_rfc3339(),
            metadata: session.metadata.clone(),
            last_consolidated: session.last_consolidated,
        };
        writeln!(file, "{}", serde_json::to_string(&meta)?)?;

        // Write each message
        for msg in &session.messages {
            writeln!(file, "{}", serde_json::to_string(msg)?)?;
        }

        Ok(())
    }

    /// Remove a session from the in-memory cache.
    pub fn invalidate(&mut self, key: &str) {
        self.sessions.remove(key);
    }

    /// List all sessions by reading metadata lines from JSONL files.
    pub fn list_sessions(&self) -> Vec<SessionInfo> {
        let mut sessions = Vec::new();

        let entries = match std::fs::read_dir(&self.sessions_dir) {
            Ok(e) => e,
            Err(_) => return sessions,
        };

        for entry in entries.flatten() {
            let path = entry.path();
            if path.extension().and_then(|e| e.to_str()) != Some("jsonl") {
                continue;
            }

            let file = match std::fs::File::open(&path) {
                Ok(f) => f,
                Err(_) => continue,
            };
            let mut reader = std::io::BufReader::new(file);
            let mut first_line = String::new();
            if reader.read_line(&mut first_line).is_err() || first_line.is_empty() {
                continue;
            }

            let data: serde_json::Value = match serde_json::from_str(first_line.trim()) {
                Ok(d) => d,
                Err(_) => continue,
            };

            if data.get("_type").and_then(|v| v.as_str()) == Some("metadata") {
                let key = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("")
                    .replace('_', ":");
                sessions.push(SessionInfo {
                    key,
                    created_at: data
                        .get("created_at")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    updated_at: data
                        .get("updated_at")
                        .and_then(|v| v.as_str())
                        .unwrap_or("")
                        .to_string(),
                    path: path.to_string_lossy().to_string(),
                });
            }
        }

        sessions.sort_by(|a, b| b.updated_at.cmp(&a.updated_at));
        sessions
    }
}

/// Summary info for a session (for listing).
#[derive(Debug, Clone)]
pub struct SessionInfo {
    pub key: String,
    pub created_at: String,
    pub updated_at: String,
    pub path: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_session_new() {
        let s = Session::new("test:chat".into());
        assert_eq!(s.key, "test:chat");
        assert!(s.messages.is_empty());
        assert_eq!(s.last_consolidated, 0);
        assert!(s.metadata.is_empty());
    }

    #[test]
    fn test_add_message() {
        let mut s = Session::new("k".into());
        s.add_message("user", "hello");
        s.add_message("assistant", "hi there");

        assert_eq!(s.messages.len(), 2);
        assert_eq!(s.messages[0].role, "user");
        assert_eq!(s.messages[0].content, "hello");
        assert!(s.messages[0].timestamp.is_some());
        assert!(s.messages[0].tools_used.is_none());
        assert_eq!(s.messages[1].role, "assistant");
    }

    #[test]
    fn test_add_message_with_tools() {
        let mut s = Session::new("k".into());
        s.add_message_with_tools("assistant", "done", vec!["read_file".into(), "exec".into()]);
        s.add_message_with_tools("assistant", "simple", vec![]);

        assert_eq!(
            s.messages[0].tools_used,
            Some(vec!["read_file".into(), "exec".into()])
        );
        // Empty vec should become None
        assert!(s.messages[1].tools_used.is_none());
    }

    #[test]
    fn test_add_message_full_with_reasoning() {
        let mut s = Session::new("k".into());
        s.add_message_full(
            "assistant",
            "answer",
            vec!["web_fetch".into()],
            Some("I thought about it deeply".into()),
        );

        assert_eq!(
            s.messages[0].reasoning_content,
            Some("I thought about it deeply".into())
        );
        assert_eq!(s.messages[0].tools_used, Some(vec!["web_fetch".into()]));
    }

    #[test]
    fn test_add_message_full_no_reasoning() {
        let mut s = Session::new("k".into());
        s.add_message_full("assistant", "answer", vec![], None);

        assert!(s.messages[0].reasoning_content.is_none());
        assert!(s.messages[0].tools_used.is_none());
    }

    #[test]
    fn test_get_history_windowing() {
        let mut s = Session::new("k".into());
        for i in 0..10 {
            s.add_message("user", &format!("msg {i}"));
        }

        let h = s.get_history(3);
        assert_eq!(h.len(), 3);
        assert_eq!(h[0].content, "msg 7");
        assert_eq!(h[2].content, "msg 9");

        // Window larger than messages
        let h = s.get_history(100);
        assert_eq!(h.len(), 10);
        assert_eq!(h[0].content, "msg 0");

        // Empty session
        let empty = Session::new("e".into());
        assert!(empty.get_history(5).is_empty());
    }

    #[test]
    fn test_clear() {
        let mut s = Session::new("k".into());
        s.add_message("user", "hello");
        s.last_consolidated = 5;
        let before = s.updated_at;

        std::thread::sleep(std::time::Duration::from_millis(10));
        s.clear();

        assert!(s.messages.is_empty());
        assert_eq!(s.last_consolidated, 0);
        assert!(s.updated_at >= before);
    }

    #[test]
    fn test_updated_at_tracks_mutations() {
        let mut s = Session::new("k".into());
        let t0 = s.updated_at;
        std::thread::sleep(std::time::Duration::from_millis(10));

        s.add_message("user", "hi");
        let t1 = s.updated_at;
        assert!(t1 > t0);
    }

    #[test]
    fn test_session_manager_save_load_roundtrip() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = SessionManager::new(dir.path().to_path_buf());

        let session = mgr.get_or_create("telegram:12345");
        session.add_message("user", "hello world");
        session.add_message_full(
            "assistant",
            "hi!",
            vec!["exec".into()],
            Some("reasoning here".into()),
        );
        session
            .metadata
            .insert("test".into(), serde_json::json!("value"));
        session.last_consolidated = 1;
        mgr.save("telegram:12345").unwrap();

        // Load into a fresh manager
        let mut mgr2 = SessionManager::new(dir.path().to_path_buf());
        let loaded = mgr2.get_or_create("telegram:12345");

        assert_eq!(loaded.messages.len(), 2);
        assert_eq!(loaded.messages[0].role, "user");
        assert_eq!(loaded.messages[0].content, "hello world");
        assert_eq!(loaded.messages[1].role, "assistant");
        assert_eq!(loaded.messages[1].content, "hi!");
        assert_eq!(loaded.messages[1].tools_used, Some(vec!["exec".into()]));
        assert_eq!(
            loaded.messages[1].reasoning_content,
            Some("reasoning here".into())
        );
        assert_eq!(loaded.last_consolidated, 1);
        assert_eq!(
            loaded.metadata.get("test"),
            Some(&serde_json::json!("value"))
        );
    }

    #[test]
    fn test_session_path_escaping() {
        let dir = tempfile::tempdir().unwrap();
        let mgr = SessionManager::new(dir.path().to_path_buf());

        let path = mgr.session_path("telegram:12345");
        assert!(path.to_string_lossy().contains("telegram_12345.jsonl"));
    }

    #[test]
    fn test_missing_session_creates_new() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = SessionManager::new(dir.path().to_path_buf());

        let session = mgr.get_or_create("nonexistent:key");
        assert!(session.messages.is_empty());
        assert_eq!(session.key, "nonexistent:key");
    }

    #[test]
    fn test_invalidate_removes_from_cache() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = SessionManager::new(dir.path().to_path_buf());

        mgr.get_or_create("k");
        assert!(mgr.sessions.contains_key("k"));

        mgr.invalidate("k");
        assert!(!mgr.sessions.contains_key("k"));
    }

    #[test]
    fn test_list_sessions() {
        let dir = tempfile::tempdir().unwrap();
        let mut mgr = SessionManager::new(dir.path().to_path_buf());

        mgr.get_or_create("cli:interactive");
        mgr.save("cli:interactive").unwrap();

        mgr.get_or_create("telegram:99");
        mgr.save("telegram:99").unwrap();

        let list = mgr.list_sessions();
        assert_eq!(list.len(), 2);
        // Keys should have colons restored
        let keys: Vec<&str> = list.iter().map(|s| s.key.as_str()).collect();
        assert!(keys.contains(&"cli:interactive"));
        assert!(keys.contains(&"telegram:99"));
    }

    #[test]
    fn test_malformed_jsonl_lines_skipped() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("bad_session.jsonl");
        std::fs::write(
            &path,
            r#"{"_type":"metadata","created_at":"2025-01-01T00:00:00Z","updated_at":"2025-01-01T00:00:00Z","metadata":{},"last_consolidated":0}
not valid json
{"role":"user","content":"hello"}
{"broken
{"role":"assistant","content":"hi"}
"#,
        )
        .unwrap();

        let mut mgr = SessionManager::new(dir.path().to_path_buf());
        let session = mgr.get_or_create("bad_session");
        // Should have loaded 2 valid messages, skipping the 2 malformed lines
        assert_eq!(session.messages.len(), 2);
        assert_eq!(session.messages[0].content, "hello");
        assert_eq!(session.messages[1].content, "hi");
    }
}
