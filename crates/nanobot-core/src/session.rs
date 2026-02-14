use std::collections::HashMap;
use std::path::PathBuf;

use anyhow::Result;
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

/// A conversation session.
pub struct Session {
    pub key: String,
    pub messages: Vec<Message>,
    pub last_consolidated: usize,
}

impl Session {
    pub fn new(key: String) -> Self {
        Self {
            key,
            messages: Vec::new(),
            last_consolidated: 0,
        }
    }

    pub fn add_message(&mut self, role: &str, content: &str) {
        self.messages.push(Message {
            role: role.into(),
            content: content.into(),
            timestamp: None,
            tools_used: None,
        });
    }

    pub fn get_history(&self, max_messages: usize) -> &[Message] {
        let start = self.messages.len().saturating_sub(max_messages);
        &self.messages[start..]
    }

    pub fn clear(&mut self) {
        self.messages.clear();
        self.last_consolidated = 0;
    }
}

/// Manages multiple sessions with JSONL persistence.
pub struct SessionManager {
    sessions_dir: PathBuf,
    cache: HashMap<String, Session>,
}

impl SessionManager {
    pub fn new(sessions_dir: PathBuf) -> Self {
        Self {
            sessions_dir,
            cache: HashMap::new(),
        }
    }

    pub fn get_or_create(&mut self, key: &str) -> &mut Session {
        if !self.cache.contains_key(key) {
            // TODO: load from disk if exists
            self.cache.insert(key.into(), Session::new(key.into()));
        }
        self.cache.get_mut(key).unwrap()
    }

    pub fn save(&self, _session: &Session) -> Result<()> {
        // TODO: persist to JSONL
        Ok(())
    }
}
