use std::collections::HashMap;
use std::io::{BufRead, Write};
use std::path::PathBuf;

use anyhow::Result;
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
        });
        self.updated_at = Utc::now();
    }

    pub fn add_message_with_tools(&mut self, role: &str, content: &str, tools: Vec<String>) {
        self.messages.push(Message {
            role: role.into(),
            content: content.into(),
            timestamp: Some(Local::now().format("%Y-%m-%dT%H:%M:%S").to_string()),
            tools_used: if tools.is_empty() { None } else { Some(tools) },
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
    cache: HashMap<String, Session>,
}

impl SessionManager {
    pub fn new(sessions_dir: PathBuf) -> Self {
        std::fs::create_dir_all(&sessions_dir).ok();
        Self {
            sessions_dir,
            cache: HashMap::new(),
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
        if !self.cache.contains_key(key) {
            let session = self.load(key).unwrap_or_else(|| Session::new(key.into()));
            self.cache.insert(key.into(), session);
        }
        self.cache.get_mut(key).unwrap()
    }

    /// Load a session from its JSONL file.
    fn load(&self, key: &str) -> Option<Session> {
        let path = self.session_path(key);
        if !path.exists() {
            return None;
        }

        let file = std::fs::File::open(&path).ok()?;
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

        Some(Session {
            key: key.into(),
            messages,
            created_at,
            updated_at: Utc::now(),
            metadata,
            last_consolidated,
        })
    }

    /// Save a session to its JSONL file.
    pub fn save(&mut self, key: &str) -> Result<()> {
        let session = self
            .cache
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
        self.cache.remove(key);
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
