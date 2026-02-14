use std::path::{Path, PathBuf};

use anyhow::Result;

use crate::Config;

/// Resolve workspace path, expanding ~ to home directory.
pub fn resolve_workspace(path: &str) -> PathBuf {
    if path.starts_with("~/") || path == "~" {
        if let Some(home) = dirs::home_dir() {
            return home.join(path.strip_prefix("~/").unwrap_or(""));
        }
    }
    PathBuf::from(path)
}

/// Find the config file by searching standard locations.
pub fn find_config_path() -> PathBuf {
    // 1. Current directory
    let local = Path::new("config.json");
    if local.exists() {
        return local.to_path_buf();
    }

    // 2. ~/.nanobot/config.json
    if let Some(home) = dirs::home_dir() {
        let home_config = home.join(".nanobot").join("config.json");
        if home_config.exists() {
            return home_config;
        }
    }

    // Default: ~/.nanobot/config.json (will use defaults if missing)
    dirs::home_dir()
        .map(|h| h.join(".nanobot").join("config.json"))
        .unwrap_or_else(|| PathBuf::from("config.json"))
}

/// Load configuration from a JSON file.
pub fn load_config(path: &Path) -> Result<Config> {
    if path.exists() {
        let contents = std::fs::read_to_string(path)?;
        let config: Config = serde_json::from_str(&contents)?;
        Ok(config)
    } else {
        Ok(Config::default())
    }
}

/// Save configuration to a JSON file.
pub fn save_config(path: &Path, config: &Config) -> Result<()> {
    let contents = serde_json::to_string_pretty(config)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(path, contents)?;
    Ok(())
}
