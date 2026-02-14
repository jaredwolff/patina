/// Manages spawning of background agent instances.
pub struct SubagentManager;

impl SubagentManager {
    pub fn new() -> Self {
        Self
    }

    /// Spawn a background agent task.
    pub async fn spawn(&self, _task: &str, _label: &str) -> anyhow::Result<()> {
        // TODO: create isolated agent loop in tokio::spawn
        todo!("subagent spawning not yet implemented")
    }
}

impl Default for SubagentManager {
    fn default() -> Self {
        Self::new()
    }
}
