use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;
use tokio::sync::{Mutex, RwLock};

use crate::cron::service::CronService;
use crate::cron::types::{CronSchedule, ScheduleKind};
use crate::tools::Tool;

/// Tool for scheduling cron jobs.
pub struct CronTool {
    service: Arc<Mutex<CronService>>,
    default_channel: Arc<RwLock<String>>,
    default_chat_id: Arc<RwLock<String>>,
}

impl CronTool {
    pub fn new(service: Arc<Mutex<CronService>>) -> Self {
        Self {
            service,
            default_channel: Arc::new(RwLock::new(String::new())),
            default_chat_id: Arc::new(RwLock::new(String::new())),
        }
    }

    /// Update the default routing context for job delivery.
    pub async fn set_context(&self, channel: &str, chat_id: &str) {
        *self.default_channel.write().await = channel.to_string();
        *self.default_chat_id.write().await = chat_id.to_string();
    }
}

#[async_trait]
impl Tool for CronTool {
    fn name(&self) -> &str {
        "cron"
    }

    fn description(&self) -> &str {
        "Schedule, list, or remove recurring tasks. Supports three schedule types:\n\
         - 'every_seconds': Run every N seconds (e.g. every 3600 = every hour)\n\
         - 'cron_expr': Standard cron expression (e.g. '0 9 * * *' = daily at 9am)\n\
         - 'at': One-time execution at an ISO datetime (e.g. '2025-01-15T14:00:00Z')\n\
         Use action 'add' to create, 'list' to view, 'remove' to delete."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "action": {
                    "type": "string",
                    "enum": ["add", "list", "remove"],
                    "description": "The action to perform"
                },
                "message": {
                    "type": "string",
                    "description": "Task message for the agent to execute (required for 'add')"
                },
                "name": {
                    "type": "string",
                    "description": "Short name for the job (required for 'add', max 30 chars)"
                },
                "every_seconds": {
                    "type": "integer",
                    "description": "Run every N seconds (for recurring schedule)"
                },
                "cron_expr": {
                    "type": "string",
                    "description": "Cron expression (e.g. '0 9 * * *' for daily at 9am)"
                },
                "at": {
                    "type": "string",
                    "description": "ISO datetime for one-time execution (e.g. '2025-01-15T14:00:00Z')"
                },
                "job_id": {
                    "type": "string",
                    "description": "Job ID (required for 'remove')"
                }
            },
            "required": ["action"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<String> {
        let action = params
            .get("action")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: action"))?;

        match action {
            "add" => self.handle_add(&params).await,
            "list" => self.handle_list().await,
            "remove" => self.handle_remove(&params).await,
            _ => Ok(format!(
                "Unknown action: {action}. Use 'add', 'list', or 'remove'."
            )),
        }
    }
}

impl CronTool {
    async fn handle_add(&self, params: &serde_json::Value) -> Result<String> {
        let message = params
            .get("message")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: message"))?;

        let name = params
            .get("name")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| &message[..message.len().min(30)]);

        // Determine schedule type
        let schedule = if let Some(secs) = params.get("every_seconds").and_then(|v| v.as_i64()) {
            CronSchedule {
                kind: ScheduleKind::Every,
                at_ms: None,
                every_ms: Some(secs * 1000),
                expr: None,
                tz: None,
            }
        } else if let Some(expr) = params.get("cron_expr").and_then(|v| v.as_str()) {
            CronSchedule {
                kind: ScheduleKind::Cron,
                at_ms: None,
                every_ms: None,
                expr: Some(expr.to_string()),
                tz: None,
            }
        } else if let Some(at_str) = params.get("at").and_then(|v| v.as_str()) {
            let dt = chrono::DateTime::parse_from_rfc3339(at_str)
                .map_err(|e| anyhow::anyhow!("invalid datetime '{at_str}': {e}"))?;
            CronSchedule {
                kind: ScheduleKind::At,
                at_ms: Some(dt.timestamp_millis()),
                every_ms: None,
                expr: None,
                tz: None,
            }
        } else {
            return Ok("Error: Must specify one of: every_seconds, cron_expr, or at".to_string());
        };

        let channel = {
            let ch = self.default_channel.read().await;
            if ch.is_empty() {
                None
            } else {
                Some(ch.clone())
            }
        };
        let chat_id = {
            let ci = self.default_chat_id.read().await;
            if ci.is_empty() {
                None
            } else {
                Some(ci.clone())
            }
        };

        let mut service = self.service.lock().await;
        match service.add_job(name, schedule, message, true, channel, chat_id, false) {
            Ok(job) => {
                let next = job
                    .state
                    .next_run_at_ms
                    .and_then(|ms| chrono::DateTime::from_timestamp_millis(ms))
                    .map(|dt| dt.format("%Y-%m-%d %H:%M:%S UTC").to_string())
                    .unwrap_or_else(|| "N/A".to_string());

                Ok(format!(
                    "Job '{}' created (ID: {}). Next run: {}",
                    job.name, job.id, next
                ))
            }
            Err(e) => Ok(format!("Failed to create job: {e}")),
        }
    }

    async fn handle_list(&self) -> Result<String> {
        let service = self.service.lock().await;
        let jobs = service.list_jobs(false);

        if jobs.is_empty() {
            return Ok("No active cron jobs.".to_string());
        }

        let mut output = String::from("Active cron jobs:\n");
        for job in jobs {
            let schedule_desc = match job.schedule.kind {
                ScheduleKind::Every => {
                    let secs = job.schedule.every_ms.unwrap_or(0) / 1000;
                    format!("every {secs}s")
                }
                ScheduleKind::Cron => {
                    format!("cron: {}", job.schedule.expr.as_deref().unwrap_or("?"))
                }
                ScheduleKind::At => {
                    let ts = job.schedule.at_ms.unwrap_or(0);
                    chrono::DateTime::from_timestamp_millis(ts)
                        .map(|dt| format!("at {}", dt.format("%Y-%m-%d %H:%M:%S UTC")))
                        .unwrap_or_else(|| "at ?".to_string())
                }
            };

            let next = job
                .state
                .next_run_at_ms
                .and_then(|ms| chrono::DateTime::from_timestamp_millis(ms))
                .map(|dt| dt.format("%H:%M:%S UTC").to_string())
                .unwrap_or_else(|| "N/A".to_string());

            output.push_str(&format!(
                "  [{}] '{}' — {} (next: {})\n",
                job.id, job.name, schedule_desc, next
            ));
        }

        Ok(output)
    }

    async fn handle_remove(&self, params: &serde_json::Value) -> Result<String> {
        let job_id = params
            .get("job_id")
            .and_then(|v| v.as_str())
            .ok_or_else(|| anyhow::anyhow!("missing required parameter: job_id"))?;

        let mut service = self.service.lock().await;
        if service.remove_job(job_id) {
            Ok(format!("Job {job_id} removed."))
        } else {
            Ok(format!("Job {job_id} not found."))
        }
    }
}
