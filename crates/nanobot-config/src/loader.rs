use std::path::Path;

use anyhow::Result;

use crate::Config;

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
