use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Persona {
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub preamble: String,
    #[serde(default)]
    pub model_tier: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub color: String,
}

/// Manages personas stored in a standalone JSON file.
pub struct PersonaStore {
    path: PathBuf,
    personas: HashMap<String, Persona>,
}

impl PersonaStore {
    /// Load personas from disk. Missing or empty file = empty map.
    pub fn load(path: &Path) -> Self {
        let personas = match std::fs::read_to_string(path) {
            Ok(contents) if !contents.trim().is_empty() => serde_json::from_str(&contents)
                .unwrap_or_else(|e| {
                    tracing::warn!("Failed to parse personas file {}: {e}", path.display());
                    HashMap::new()
                }),
            _ => HashMap::new(),
        };
        Self {
            path: path.to_path_buf(),
            personas,
        }
    }

    /// Save current personas to disk.
    pub fn save(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&self.personas)?;
        std::fs::write(&self.path, json)?;
        Ok(())
    }

    /// List all personas as (key, persona) pairs.
    pub fn list(&self) -> &HashMap<String, Persona> {
        &self.personas
    }

    /// Get a persona by key.
    pub fn get(&self, key: &str) -> Option<&Persona> {
        self.personas.get(key)
    }

    /// Create or update a persona.
    pub fn upsert(&mut self, key: String, persona: Persona) -> Result<()> {
        self.personas.insert(key, persona);
        self.save()
    }

    /// Remove a persona. Returns true if it existed.
    pub fn remove(&mut self, key: &str) -> Result<bool> {
        let existed = self.personas.remove(key).is_some();
        if existed {
            self.save()?;
        }
        Ok(existed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn test_load_missing_file() {
        let dir = TempDir::new().unwrap();
        let store = PersonaStore::load(&dir.path().join("missing.json"));
        assert!(store.list().is_empty());
    }

    #[test]
    fn test_load_empty_file() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("personas.json");
        std::fs::write(&path, "").unwrap();
        let store = PersonaStore::load(&path);
        assert!(store.list().is_empty());
    }

    #[test]
    fn test_roundtrip() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("personas.json");

        let mut store = PersonaStore::load(&path);
        store
            .upsert(
                "coder".into(),
                Persona {
                    name: "Code Assistant".into(),
                    description: "Helps with code".into(),
                    preamble: "You are a coding assistant.".into(),
                    model_tier: "coding".into(),
                    color: String::new(),
                },
            )
            .unwrap();

        // Reload from disk
        let store2 = PersonaStore::load(&path);
        assert_eq!(store2.list().len(), 1);
        let p = store2.get("coder").unwrap();
        assert_eq!(p.name, "Code Assistant");
        assert_eq!(p.preamble, "You are a coding assistant.");
        assert_eq!(p.model_tier, "coding");
    }

    #[test]
    fn test_remove() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("personas.json");

        let mut store = PersonaStore::load(&path);
        store
            .upsert(
                "test".into(),
                Persona {
                    name: "Test".into(),
                    description: String::new(),
                    preamble: String::new(),
                    model_tier: String::new(),
                    color: String::new(),
                },
            )
            .unwrap();
        assert_eq!(store.list().len(), 1);

        assert!(store.remove("test").unwrap());
        assert_eq!(store.list().len(), 0);
        assert!(!store.remove("test").unwrap());

        // Verify persisted
        let store2 = PersonaStore::load(&path);
        assert!(store2.list().is_empty());
    }

    #[test]
    fn test_upsert_overwrites() {
        let dir = TempDir::new().unwrap();
        let path = dir.path().join("personas.json");

        let mut store = PersonaStore::load(&path);
        store
            .upsert(
                "x".into(),
                Persona {
                    name: "V1".into(),
                    description: String::new(),
                    preamble: String::new(),
                    model_tier: String::new(),
                    color: String::new(),
                },
            )
            .unwrap();
        store
            .upsert(
                "x".into(),
                Persona {
                    name: "V2".into(),
                    description: String::new(),
                    preamble: String::new(),
                    model_tier: String::new(),
                    color: String::new(),
                },
            )
            .unwrap();

        assert_eq!(store.list().len(), 1);
        assert_eq!(store.get("x").unwrap().name, "V2");
    }
}
