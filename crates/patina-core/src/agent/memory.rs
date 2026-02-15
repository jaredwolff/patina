use std::path::PathBuf;

use anyhow::Result;

/// Two-layer memory system: MEMORY.md (facts) + HISTORY.md (event log).
#[derive(Clone)]
pub struct MemoryStore {
    memory_path: PathBuf,
    history_path: PathBuf,
}

impl MemoryStore {
    pub fn new(workspace: &std::path::Path) -> Self {
        Self {
            memory_path: workspace.join("memory").join("MEMORY.md"),
            history_path: workspace.join("memory").join("HISTORY.md"),
        }
    }

    pub fn memory_path(&self) -> &std::path::Path {
        &self.memory_path
    }

    pub fn history_path(&self) -> &std::path::Path {
        &self.history_path
    }

    pub fn read_long_term(&self) -> Result<String> {
        if self.memory_path.exists() {
            Ok(std::fs::read_to_string(&self.memory_path)?)
        } else {
            Ok(String::new())
        }
    }

    pub fn write_long_term(&self, content: &str) -> Result<()> {
        if let Some(parent) = self.memory_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(std::fs::write(&self.memory_path, content)?)
    }

    pub fn append_history(&self, entry: &str) -> Result<()> {
        use std::io::Write;
        if let Some(parent) = self.history_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.history_path)?;
        writeln!(file, "\n{entry}")?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_path_construction() {
        let ws = std::path::Path::new("/tmp/workspace");
        let store = MemoryStore::new(ws);
        assert_eq!(
            store.memory_path,
            PathBuf::from("/tmp/workspace/memory/MEMORY.md")
        );
        assert_eq!(
            store.history_path,
            PathBuf::from("/tmp/workspace/memory/HISTORY.md")
        );
    }

    #[test]
    fn test_read_long_term_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path());
        let content = store.read_long_term().unwrap();
        assert_eq!(content, "");
    }

    #[test]
    fn test_write_and_read_long_term() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path());

        store.write_long_term("# Memory\nUser likes Rust").unwrap();
        let content = store.read_long_term().unwrap();
        assert_eq!(content, "# Memory\nUser likes Rust");
    }

    #[test]
    fn test_write_creates_directories() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path());
        // memory/ subdir doesn't exist yet
        assert!(!dir.path().join("memory").exists());

        store.write_long_term("test").unwrap();
        assert!(dir.path().join("memory").exists());
        assert!(dir.path().join("memory/MEMORY.md").exists());
    }

    #[test]
    fn test_append_history() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path());

        store.append_history("[2025-01-01] First entry").unwrap();
        store.append_history("[2025-01-02] Second entry").unwrap();

        let content = std::fs::read_to_string(&store.history_path).unwrap();
        assert!(content.contains("[2025-01-01] First entry"));
        assert!(content.contains("[2025-01-02] Second entry"));
    }

    #[test]
    fn test_overwrite_long_term() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path());

        store.write_long_term("old content").unwrap();
        store.write_long_term("new content").unwrap();

        let content = store.read_long_term().unwrap();
        assert_eq!(content, "new content");
    }

    #[test]
    fn test_unicode_content() {
        let dir = tempfile::tempdir().unwrap();
        let store = MemoryStore::new(dir.path());

        let text = "用户喜欢 Rust 🦀 и Python 🐍";
        store.write_long_term(text).unwrap();
        assert_eq!(store.read_long_term().unwrap(), text);
    }
}
