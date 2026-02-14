use std::path::PathBuf;

use anyhow::Result;

/// Two-layer memory system: MEMORY.md (facts) + HISTORY.md (event log).
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
