use std::path::{Path, PathBuf};
use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;
use regex::Regex;
use tokio::process::Command;

use super::Tool;

/// Shell command execution tool with safety guards.
pub struct ExecTool {
    timeout: Duration,
    working_dir: PathBuf,
    deny_patterns: Vec<Regex>,
    restrict_to_workspace: bool,
    posix_path_re: Regex,
}

impl ExecTool {
    pub fn new(working_dir: PathBuf, timeout_secs: u64, restrict_to_workspace: bool) -> Self {
        let deny_patterns: Vec<Regex> = [
            r"\brm\s+-[rf]{1,2}\b",
            r"\bdel\s+/[fq]\b",
            r"\brmdir\s+/s\b",
            r"\b(format|mkfs|diskpart)\b",
            r"\bdd\s+if=",
            r">\s*/dev/sd",
            r"\b(shutdown|reboot|poweroff)\b",
            r":\(\)\s*\{.*\};\s*:",
        ]
        .iter()
        .filter_map(|p| Regex::new(p).ok())
        .collect();

        let posix_path_re = Regex::new(r#"(?:^|[\s|>])(/[^\s"'>]+)"#).unwrap();

        Self {
            timeout: Duration::from_secs(timeout_secs),
            working_dir,
            deny_patterns,
            restrict_to_workspace,
            posix_path_re,
        }
    }

    fn guard_command(&self, command: &str, cwd: &Path) -> Option<String> {
        let lower = command.to_lowercase();

        // Check deny patterns
        for re in &self.deny_patterns {
            if re.is_match(&lower) {
                return Some(
                    "Error: Command blocked by safety guard (dangerous pattern detected)".into(),
                );
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
            for cap in self.posix_path_re.captures_iter(command) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn make_tool(restrict: bool) -> ExecTool {
        let dir = std::env::temp_dir().join("nanobot_shell_test");
        std::fs::create_dir_all(&dir).ok();
        ExecTool::new(dir, 10, restrict)
    }

    // --- Safety guard tests ---

    #[test]
    fn test_guard_allows_safe_commands() {
        let tool = make_tool(false);
        assert!(tool
            .guard_command("echo hello", &tool.working_dir)
            .is_none());
        assert!(tool.guard_command("ls -la", &tool.working_dir).is_none());
        assert!(tool
            .guard_command("cat /etc/hostname", &tool.working_dir)
            .is_none());
    }

    #[test]
    fn test_guard_blocks_rm_rf() {
        let tool = make_tool(false);
        let result = tool.guard_command("rm -rf /", &tool.working_dir);
        assert!(result.is_some());
        assert!(result.unwrap().contains("blocked"));
    }

    #[test]
    fn test_guard_blocks_rm_f() {
        let tool = make_tool(false);
        assert!(tool
            .guard_command("rm -f important.db", &tool.working_dir)
            .is_some());
    }

    #[test]
    fn test_guard_blocks_dd() {
        let tool = make_tool(false);
        assert!(tool
            .guard_command("dd if=/dev/zero of=/dev/sda", &tool.working_dir)
            .is_some());
    }

    #[test]
    fn test_guard_blocks_shutdown() {
        let tool = make_tool(false);
        assert!(tool
            .guard_command("shutdown -h now", &tool.working_dir)
            .is_some());
        assert!(tool.guard_command("reboot", &tool.working_dir).is_some());
    }

    #[test]
    fn test_guard_blocks_fork_bomb() {
        let tool = make_tool(false);
        assert!(tool
            .guard_command(":() { :|:& }; :", &tool.working_dir)
            .is_some());
    }

    #[test]
    fn test_guard_blocks_path_traversal_when_restricted() {
        let tool = make_tool(true);
        let result = tool.guard_command("cat ../../../etc/passwd", &tool.working_dir);
        assert!(result.is_some());
        assert!(result.unwrap().contains("path traversal"));
    }

    #[test]
    fn test_guard_allows_path_traversal_when_unrestricted() {
        let tool = make_tool(false);
        assert!(tool
            .guard_command("cat ../something", &tool.working_dir)
            .is_none());
    }

    // --- Execution tests ---

    #[tokio::test]
    async fn test_exec_simple_command() {
        let tool = make_tool(false);
        let result = tool
            .execute(serde_json::json!({"command": "echo hello"}))
            .await
            .unwrap();
        assert_eq!(result.trim(), "hello");
    }

    #[tokio::test]
    async fn test_exec_captures_stderr() {
        let tool = make_tool(false);
        let result = tool
            .execute(serde_json::json!({"command": "echo err >&2"}))
            .await
            .unwrap();
        assert!(result.contains("STDERR:"));
        assert!(result.contains("err"));
    }

    #[tokio::test]
    async fn test_exec_nonzero_exit() {
        let tool = make_tool(false);
        let result = tool
            .execute(serde_json::json!({"command": "exit 42"}))
            .await
            .unwrap();
        assert!(result.contains("Exit code: 42"));
    }

    #[tokio::test]
    async fn test_exec_no_output() {
        let tool = make_tool(false);
        let result = tool
            .execute(serde_json::json!({"command": "true"}))
            .await
            .unwrap();
        assert_eq!(result, "(no output)");
    }

    #[tokio::test]
    async fn test_exec_timeout() {
        let dir = std::env::temp_dir().join("nanobot_shell_test");
        std::fs::create_dir_all(&dir).ok();
        let tool = ExecTool::new(dir, 1, false); // 1 second timeout

        let result = tool
            .execute(serde_json::json!({"command": "sleep 10"}))
            .await
            .unwrap();
        assert!(result.contains("timed out"));
    }

    #[tokio::test]
    async fn test_exec_blocked_command() {
        let tool = make_tool(false);
        let result = tool
            .execute(serde_json::json!({"command": "rm -rf /"}))
            .await
            .unwrap();
        assert!(result.contains("blocked"));
    }
}
