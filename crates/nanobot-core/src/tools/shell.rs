use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use tokio::process::Command;

use super::Tool;

/// Shell command execution tool with safety guards.
pub struct ExecTool {
    timeout: Duration,
    working_dir: PathBuf,
    deny_patterns: Vec<String>,
    restrict_to_workspace: bool,
}

impl ExecTool {
    pub fn new(working_dir: PathBuf, timeout_secs: u64, restrict_to_workspace: bool) -> Self {
        Self {
            timeout: Duration::from_secs(timeout_secs),
            working_dir,
            deny_patterns: vec![
                r"\brm\s+-[rf]{1,2}\b".into(),
                r"\bdel\s+/[fq]\b".into(),
                r"\brmdir\s+/s\b".into(),
                r"\b(format|mkfs|diskpart)\b".into(),
                r"\bdd\s+if=".into(),
                r">\s*/dev/sd".into(),
                r"\b(shutdown|reboot|poweroff)\b".into(),
                r":\(\)\s*\{.*\};\s*:".into(),
            ],
            restrict_to_workspace,
        }
    }

    fn guard_command(&self, command: &str, cwd: &Path) -> Option<String> {
        let lower = command.to_lowercase();

        // Check deny patterns
        for pattern in &self.deny_patterns {
            if let Ok(re) = regex::Regex::new(pattern) {
                if re.is_match(&lower) {
                    return Some(
                        "Error: Command blocked by safety guard (dangerous pattern detected)"
                            .into(),
                    );
                }
            }
        }

        // Check workspace restriction
        if self.restrict_to_workspace {
            if command.contains("../") || command.contains("..\\") {
                return Some(
                    "Error: Command blocked by safety guard (path traversal detected)".into(),
                );
            }

            let cwd_resolved = cwd.canonicalize().unwrap_or_else(|_| cwd.to_path_buf());

            // Check for absolute paths outside workspace
            let posix_re = regex::Regex::new(r#"(?:^|[\s|>])(/[^\s"'>]+)"#).unwrap();
            for cap in posix_re.captures_iter(command) {
                if let Some(m) = cap.get(1) {
                    let p = Path::new(m.as_str());
                    if p.is_absolute() {
                        let resolved = p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
                        if !resolved.starts_with(&cwd_resolved) {
                            return Some(
                                "Error: Command blocked by safety guard (path outside working dir)"
                                    .into(),
                            );
                        }
                    }
                }
            }
        }

        None
    }
}

#[async_trait]
impl Tool for ExecTool {
    fn name(&self) -> &str {
        "exec"
    }

    fn description(&self) -> &str {
        "Execute a shell command and return its output. Use with caution."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The shell command to execute"
                },
                "working_dir": {
                    "type": "string",
                    "description": "Optional working directory for the command"
                }
            },
            "required": ["command"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<String> {
        let command = params
            .get("command")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: command"))?;

        let cwd = params
            .get("working_dir")
            .and_then(|v| v.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.working_dir.clone());

        // Safety check
        if let Some(err) = self.guard_command(command, &cwd) {
            return Ok(err);
        }

        let result = tokio::time::timeout(
            self.timeout,
            Command::new("sh")
                .arg("-c")
                .arg(command)
                .current_dir(&cwd)
                .output(),
        )
        .await;

        match result {
            Ok(Ok(output)) => {
                let mut parts = Vec::new();

                let stdout = String::from_utf8_lossy(&output.stdout);
                if !stdout.is_empty() {
                    parts.push(stdout.to_string());
                }

                let stderr = String::from_utf8_lossy(&output.stderr);
                if !stderr.trim().is_empty() {
                    parts.push(format!("STDERR:\n{stderr}"));
                }

                if !output.status.success() {
                    parts.push(format!(
                        "\nExit code: {}",
                        output.status.code().unwrap_or(-1)
                    ));
                }

                let result = if parts.is_empty() {
                    "(no output)".to_string()
                } else {
                    parts.join("\n")
                };

                // Truncate very long output
                let max_len = 10_000;
                if result.len() > max_len {
                    Ok(format!(
                        "{}\n... (truncated, {} more chars)",
                        &result[..max_len],
                        result.len() - max_len
                    ))
                } else {
                    Ok(result)
                }
            }
            Ok(Err(e)) => Ok(format!("Error executing command: {e}")),
            Err(_) => Ok(format!(
                "Error: Command timed out after {} seconds",
                self.timeout.as_secs()
            )),
        }
    }
}
