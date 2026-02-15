use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::bus::InboundMessage;

const DEFAULT_INTERVAL_SECS: u64 = 30 * 60; // 30 minutes

const HEARTBEAT_PROMPT: &str = "\
Read HEARTBEAT.md in your workspace (if it exists). \
Follow any instructions or tasks listed there. \
If nothing needs attention, reply with just: HEARTBEAT_OK";

/// Service that periodically checks HEARTBEAT.md and triggers agent action.
pub struct HeartbeatService {
    workspace: PathBuf,
    interval: Duration,
    inbound_tx: mpsc::Sender<InboundMessage>,
    handle: Option<JoinHandle<()>>,
}

impl HeartbeatService {
    pub fn new(
        workspace: PathBuf,
        inbound_tx: mpsc::Sender<InboundMessage>,
        interval_secs: Option<u64>,
    ) -> Self {
        Self {
            workspace,
            interval: Duration::from_secs(interval_secs.unwrap_or(DEFAULT_INTERVAL_SECS)),
            inbound_tx,
            handle: None,
        }
    }

    /// Start the heartbeat background loop.
    pub fn start(&mut self) {
        let workspace = self.workspace.clone();
        let interval = self.interval;
        let inbound_tx = self.inbound_tx.clone();

        self.handle = Some(tokio::spawn(async move {
            info!(
                "Heartbeat service started (interval: {}s)",
                interval.as_secs()
            );

            loop {
                tokio::time::sleep(interval).await;

                if let Err(e) = tick(&workspace, &inbound_tx).await {
                    warn!("Heartbeat tick error: {e}");
                }
            }
        }));
    }

    /// Stop the heartbeat loop.
    pub fn stop(&mut self) {
        if let Some(handle) = self.handle.take() {
            handle.abort();
            info!("Heartbeat service stopped");
        }
    }

    /// Trigger a heartbeat check immediately (for testing).
    pub async fn trigger_now(&self) -> anyhow::Result<()> {
        tick(&self.workspace, &self.inbound_tx).await
    }

    /// Path to the heartbeat file.
    pub fn heartbeat_file(&self) -> PathBuf {
        self.workspace.join("HEARTBEAT.md")
    }
}

/// Run a single heartbeat tick.
async fn tick(workspace: &Path, inbound_tx: &mpsc::Sender<InboundMessage>) -> anyhow::Result<()> {
    let heartbeat_path = workspace.join("HEARTBEAT.md");

    if !heartbeat_path.exists() {
        info!("Heartbeat: no HEARTBEAT.md found, skipping");
        return Ok(());
    }

    let content = std::fs::read_to_string(&heartbeat_path)?;

    if is_heartbeat_empty(&content) {
        info!("Heartbeat: HEARTBEAT.md has no actionable content, skipping");
        return Ok(());
    }

    info!("Heartbeat: found tasks in HEARTBEAT.md, triggering agent");

    let msg = InboundMessage {
        channel: "system".to_string(),
        sender_id: "heartbeat".to_string(),
        chat_id: "system:heartbeat".to_string(),
        content: HEARTBEAT_PROMPT.to_string(),
        media: Vec::new(),
        metadata: HashMap::new(),
    };

    inbound_tx
        .send(msg)
        .await
        .map_err(|e| anyhow::anyhow!("Failed to send heartbeat message: {e}"))?;

    Ok(())
}

/// Check if HEARTBEAT.md contains only structural content (no actionable tasks).
///
/// Skips: empty lines, lines starting with #, lines starting with <!--,
/// and checkbox-only lines (- [ ], * [ ], - [x], * [x]).
fn is_heartbeat_empty(content: &str) -> bool {
    const SKIP_PATTERNS: &[&str] = &["- [ ]", "* [ ]", "- [x]", "* [x]"];

    for line in content.lines() {
        let trimmed = line.trim();

        // Skip empty lines
        if trimmed.is_empty() {
            continue;
        }

        // Skip markdown headers
        if trimmed.starts_with('#') {
            continue;
        }

        // Skip HTML comments
        if trimmed.starts_with("<!--") {
            continue;
        }

        // Skip empty/completed checkboxes (exact match)
        if SKIP_PATTERNS.contains(&trimmed) {
            continue;
        }

        // Found actual content
        return false;
    }

    true
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;
    use tokio::sync::mpsc;
    use tokio::time::{timeout, Duration};

    #[test]
    fn test_empty_heartbeat() {
        assert!(is_heartbeat_empty(""));
        assert!(is_heartbeat_empty("# Header\n\n## Another\n"));
        assert!(is_heartbeat_empty("# Header\n<!-- comment -->\n"));
        assert!(is_heartbeat_empty(
            "# Heartbeat\n\n<!-- Add tasks here -->\n\n## Active\n"
        ));
    }

    #[test]
    fn test_checkbox_patterns_are_skipped() {
        // All checkbox patterns should be treated as empty
        assert!(is_heartbeat_empty("- [ ]"));
        assert!(is_heartbeat_empty("* [ ]"));
        assert!(is_heartbeat_empty("- [x]"));
        assert!(is_heartbeat_empty("* [x]"));
        assert!(is_heartbeat_empty("# Tasks\n- [ ]\n* [x]\n"));
        // Checkbox with label text is NOT empty
        assert!(!is_heartbeat_empty("- [x] Done task"));
        assert!(!is_heartbeat_empty("- [ ] Pending task"));
    }

    #[test]
    fn test_non_empty_heartbeat() {
        assert!(!is_heartbeat_empty("- Check system health\n"));
        assert!(!is_heartbeat_empty("# Tasks\n- Do something\n"));
        assert!(!is_heartbeat_empty(
            "# Heartbeat\n<!-- comment -->\n- [x] Done task\n"
        ));
    }

    #[tokio::test]
    async fn trigger_now_emits_inbound_message_when_actionable() {
        let dir = tempdir().unwrap();
        let heartbeat_path = dir.path().join("HEARTBEAT.md");
        std::fs::write(&heartbeat_path, "- check integrations").unwrap();

        let (tx, mut rx) = mpsc::channel(1);
        let svc = HeartbeatService::new(dir.path().to_path_buf(), tx, Some(1));
        svc.trigger_now().await.unwrap();

        let msg = timeout(Duration::from_millis(200), rx.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(msg.channel, "system");
        assert_eq!(msg.sender_id, "heartbeat");
        assert_eq!(msg.chat_id, "system:heartbeat");
    }

    #[tokio::test]
    async fn trigger_now_skips_when_file_is_structurally_empty() {
        let dir = tempdir().unwrap();
        let heartbeat_path = dir.path().join("HEARTBEAT.md");
        std::fs::write(&heartbeat_path, "# Heartbeat\n- [ ]\n").unwrap();

        let (tx, mut rx) = mpsc::channel(1);
        let svc = HeartbeatService::new(dir.path().to_path_buf(), tx, Some(1));
        svc.trigger_now().await.unwrap();

        let recv = timeout(Duration::from_millis(100), rx.recv()).await;
        assert!(recv.is_err(), "no heartbeat message should be emitted");
    }
}
