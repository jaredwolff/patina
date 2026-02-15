use std::path::{Path, PathBuf};

use anyhow::{Context, Result};

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

    // 2. ~/.patina/config.json
    if let Some(home) = dirs::home_dir() {
        let home_config = home.join(".patina").join("config.json");
        if home_config.exists() {
            return home_config;
        }
    }

    // Default: ~/.patina/config.json (will use defaults if missing)
    dirs::home_dir()
        .map(|h| h.join(".patina").join("config.json"))
        .unwrap_or_else(|| PathBuf::from("config.json"))
}

/// Load configuration from a JSON file.
pub fn load_config(path: &Path) -> Result<Config> {
    if path.exists() {
        let contents = std::fs::read_to_string(path)
            .with_context(|| format!("failed to read config '{}'", path.display()))?;
        let config: Config = serde_json::from_str(&contents)
            .with_context(|| format!("failed to parse config '{}'", path.display()))?;
        Ok(config)
    } else {
        Ok(Config::default())
    }
}

/// Save configuration to a JSON file.
pub fn save_config(path: &Path, config: &Config) -> Result<()> {
    let contents = serde_json::to_string_pretty(config)?;
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).with_context(|| {
            format!(
                "failed to create config directory '{}'",
                parent.to_string_lossy()
            )
        })?;
    }
    std::fs::write(path, contents)
        .with_context(|| format!("failed to write config '{}'", path.display()))?;
    Ok(())
}
