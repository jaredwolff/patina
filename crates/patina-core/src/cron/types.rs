use serde::{Deserialize, Serialize};

/// Schedule type for a cron job.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ScheduleKind {
    At,
    Every,
    Cron,
}

/// Schedule definition for a cron job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronSchedule {
    pub kind: ScheduleKind,
    /// For "at": timestamp in milliseconds since epoch.
    pub at_ms: Option<i64>,
    /// For "every": interval in milliseconds.
    pub every_ms: Option<i64>,
    /// For "cron": cron expression (e.g. "0 9 * * *").
    pub expr: Option<String>,
    /// Timezone for cron expressions.
    pub tz: Option<String>,
}

/// What happens when a cron job fires.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronPayload {
    /// "agent_turn" (LLM processing) or "exec" (direct shell command)
    #[serde(default = "default_payload_kind")]
    pub kind: String,
    /// The message/task to execute.
    pub message: String,
    /// Whether to deliver the result to a channel.
    #[serde(default)]
    pub deliver: bool,
    /// Target channel for delivery.
    pub channel: Option<String>,
    /// Target chat_id for delivery.
    pub to: Option<String>,
}

fn default_payload_kind() -> String {
    "agent_turn".to_string()
}

/// Execution state of a cron job.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct CronJobState {
    pub next_run_at_ms: Option<i64>,
    pub last_run_at_ms: Option<i64>,
    pub last_status: Option<String>,
    pub last_error: Option<String>,
}

/// A scheduled cron job.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronJob {
    pub id: String,
    pub name: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    pub schedule: CronSchedule,
    pub payload: CronPayload,
    pub state: CronJobState,
    pub created_at_ms: i64,
    pub updated_at_ms: i64,
    #[serde(default)]
    pub delete_after_run: bool,
}

fn default_true() -> bool {
    true
}

/// Persistence format for cron jobs.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct CronStore {
    pub version: u32,
    pub jobs: Vec<CronJob>,
}

impl Default for CronStore {
    fn default() -> Self {
        Self {
            version: 1,
            jobs: Vec::new(),
        }
    }
}
