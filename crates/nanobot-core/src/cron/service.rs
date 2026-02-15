use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::Result;
use chrono::Utc;
use croner::Cron;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tracing::{info, warn};

use crate::bus::InboundMessage;
use crate::cron::types::*;

/// Service that manages scheduled cron jobs.
pub struct CronService {
    store_path: PathBuf,
    jobs: Vec<CronJob>,
    timer_handle: Option<JoinHandle<()>>,
    inbound_tx: mpsc::Sender<InboundMessage>,
}

impl CronService {
    pub fn new(store_path: PathBuf, inbound_tx: mpsc::Sender<InboundMessage>) -> Self {
        Self {
            store_path,
            jobs: Vec::new(),
            timer_handle: None,
            inbound_tx,
        }
    }

    /// Load jobs from disk and start the timer.
    pub async fn start(&mut self) -> Result<()> {
        self.load()?;
        self.arm_timer();
        info!("Cron service started with {} jobs", self.jobs.len());
        Ok(())
    }

    /// Stop the timer.
    pub fn stop(&mut self) {
        if let Some(handle) = self.timer_handle.take() {
            handle.abort();
            info!("Cron service stopped");
        }
    }

    /// List all jobs (optionally including disabled).
    pub fn list_jobs(&self, include_disabled: bool) -> Vec<&CronJob> {
        self.jobs
            .iter()
            .filter(|j| include_disabled || j.enabled)
            .collect()
    }

    /// Add a new cron job.
    pub fn add_job(
        &mut self,
        name: &str,
        schedule: CronSchedule,
        message: &str,
        deliver: bool,
        channel: Option<String>,
        to: Option<String>,
        delete_after_run: bool,
    ) -> Result<CronJob> {
        let now_ms = Utc::now().timestamp_millis();
        let id = uuid::Uuid::new_v4().to_string()[..8].to_string();

        let next_run = compute_next_run(&schedule, now_ms)?;

        let job = CronJob {
            id: id.clone(),
            name: name.chars().take(30).collect(),
            enabled: true,
            schedule,
            payload: CronPayload {
                kind: "agent_turn".to_string(),
                message: message.to_string(),
                deliver,
                channel,
                to,
            },
            state: CronJobState {
                next_run_at_ms: next_run,
                last_run_at_ms: None,
                last_status: None,
                last_error: None,
            },
            created_at_ms: now_ms,
            updated_at_ms: now_ms,
            delete_after_run,
        };

        self.jobs.push(job.clone());
        self.save()?;
        self.arm_timer();

        info!("Added cron job '{}' (id: {})", name, id);
        Ok(job)
    }

    /// Remove a job by ID.
    pub fn remove_job(&mut self, job_id: &str) -> bool {
        let len_before = self.jobs.len();
        self.jobs.retain(|j| j.id != job_id);
        let removed = self.jobs.len() < len_before;
        if removed {
            let _ = self.save();
            self.arm_timer();
            info!("Removed cron job {job_id}");
        }
        removed
    }

    /// Enable or disable a job.
    pub fn enable_job(&mut self, job_id: &str, enabled: bool) -> Option<&CronJob> {
        if let Some(job) = self.jobs.iter_mut().find(|j| j.id == job_id) {
            job.enabled = enabled;
            job.updated_at_ms = Utc::now().timestamp_millis();
            if enabled {
                let now_ms = Utc::now().timestamp_millis();
                job.state.next_run_at_ms = compute_next_run(&job.schedule, now_ms).unwrap_or(None);
            }
            let _ = self.save();
            self.arm_timer();
            self.jobs.iter().find(|j| j.id == job_id)
        } else {
            None
        }
    }

    /// Execute due jobs (called by the timer).
    async fn execute_due_jobs(
        jobs: &mut Vec<CronJob>,
        store_path: &Path,
        inbound_tx: &mpsc::Sender<InboundMessage>,
    ) {
        let now_ms = Utc::now().timestamp_millis();
        let mut jobs_to_delete = Vec::new();

        for job in jobs.iter_mut() {
            if !job.enabled {
                continue;
            }
            let next = match job.state.next_run_at_ms {
                Some(t) => t,
                None => continue,
            };
            if now_ms < next {
                continue;
            }

            info!("Executing cron job '{}' (id: {})", job.name, job.id);

            // Send the job message through the bus
            let channel = job
                .payload
                .channel
                .clone()
                .unwrap_or_else(|| "system".to_string());
            let chat_id = job.payload.to.clone().unwrap_or_else(|| "cron".to_string());

            let msg = InboundMessage {
                channel: channel.clone(),
                sender_id: "cron".to_string(),
                chat_id,
                content: job.payload.message.clone(),
                media: Vec::new(),
                metadata: {
                    let mut m = HashMap::new();
                    m.insert(
                        "cron_job_id".to_string(),
                        serde_json::Value::String(job.id.clone()),
                    );
                    m.insert(
                        "cron_job_name".to_string(),
                        serde_json::Value::String(job.name.clone()),
                    );
                    m
                },
            };

            if let Err(e) = inbound_tx.send(msg).await {
                warn!("Failed to send cron job message: {e}");
                job.state.last_status = Some("error".to_string());
                job.state.last_error = Some(format!("Failed to send: {e}"));
            } else {
                job.state.last_status = Some("ok".to_string());
                job.state.last_error = None;
            }

            job.state.last_run_at_ms = Some(now_ms);
            job.updated_at_ms = now_ms;

            // Handle one-time jobs
            if job.schedule.kind == ScheduleKind::At {
                if job.delete_after_run {
                    jobs_to_delete.push(job.id.clone());
                } else {
                    job.enabled = false;
                    job.state.next_run_at_ms = None;
                }
            } else {
                // Recompute next run for recurring jobs
                job.state.next_run_at_ms = compute_next_run(&job.schedule, now_ms).unwrap_or(None);
            }
        }

        // Delete one-time jobs that requested it
        jobs.retain(|j| !jobs_to_delete.contains(&j.id));

        // Save updated state
        let store = CronStore {
            version: 1,
            jobs: jobs.clone(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&store) {
            let _ = std::fs::write(store_path, json);
        }
    }

    /// Arm the timer to wake at the next due job.
    ///
    /// Spawns a background loop that sleeps until the next job is due,
    /// executes it, and re-arms for the next one. The loop continues
    /// until there are no more enabled jobs with a next_run_at_ms.
    fn arm_timer(&mut self) {
        // Cancel existing timer
        if let Some(handle) = self.timer_handle.take() {
            handle.abort();
        }

        let mut jobs = self.jobs.clone();
        let store_path = self.store_path.clone();
        let inbound_tx = self.inbound_tx.clone();

        self.timer_handle = Some(tokio::spawn(async move {
            loop {
                // Find the earliest next_run_at_ms
                let now_ms = Utc::now().timestamp_millis();
                let earliest = jobs
                    .iter()
                    .filter(|j| j.enabled)
                    .filter_map(|j| j.state.next_run_at_ms)
                    .min();

                let sleep_ms = match earliest {
                    Some(t) if t > now_ms => (t - now_ms) as u64,
                    Some(_) => 0,  // Already due
                    None => break, // No jobs to schedule — exit loop
                };

                if sleep_ms > 0 {
                    tokio::time::sleep(tokio::time::Duration::from_millis(sleep_ms)).await;
                }

                Self::execute_due_jobs(&mut jobs, &store_path, &inbound_tx).await;

                // If no enabled jobs remain, exit the loop
                let has_scheduled = jobs
                    .iter()
                    .any(|j| j.enabled && j.state.next_run_at_ms.is_some());
                if !has_scheduled {
                    break;
                }
            }
        }));
    }

    fn load(&mut self) -> Result<()> {
        if !self.store_path.exists() {
            self.jobs = Vec::new();
            return Ok(());
        }

        let content = std::fs::read_to_string(&self.store_path)?;
        let store: CronStore = serde_json::from_str(&content)?;
        self.jobs = store.jobs;

        // Recompute next_run for enabled recurring jobs
        let now_ms = Utc::now().timestamp_millis();
        for job in &mut self.jobs {
            if job.enabled && job.schedule.kind != ScheduleKind::At {
                job.state.next_run_at_ms = compute_next_run(&job.schedule, now_ms).unwrap_or(None);
            }
        }

        Ok(())
    }

    fn save(&self) -> Result<()> {
        if let Some(parent) = self.store_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let store = CronStore {
            version: 1,
            jobs: self.jobs.clone(),
        };
        let json = serde_json::to_string_pretty(&store)?;
        std::fs::write(&self.store_path, json)?;
        Ok(())
    }
}

/// Compute the next run time for a schedule.
fn compute_next_run(schedule: &CronSchedule, now_ms: i64) -> Result<Option<i64>> {
    match schedule.kind {
        ScheduleKind::At => {
            // One-time: return if in the future
            match schedule.at_ms {
                Some(t) if t > now_ms => Ok(Some(t)),
                _ => Ok(None),
            }
        }
        ScheduleKind::Every => {
            // Recurring interval
            match schedule.every_ms {
                Some(interval) if interval > 0 => Ok(Some(now_ms + interval)),
                _ => Ok(None),
            }
        }
        ScheduleKind::Cron => {
            // Cron expression
            let expr = schedule
                .expr
                .as_deref()
                .ok_or_else(|| anyhow::anyhow!("cron schedule missing expr"))?;

            let cron = Cron::new(expr)
                .parse()
                .map_err(|e| anyhow::anyhow!("invalid cron expression '{expr}': {e}"))?;

            let now = chrono::DateTime::from_timestamp_millis(now_ms).unwrap_or_else(|| Utc::now());

            match cron.find_next_occurrence(&now, false) {
                Ok(next) => Ok(Some(next.timestamp_millis())),
                Err(_) => Ok(None),
            }
        }
    }
}
