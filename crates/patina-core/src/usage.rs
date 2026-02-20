use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;

use anyhow::Result;
use patina_config::schema::ModelPricing;
use rusqlite::Connection;
use serde::Serialize;

/// A single LLM API call record.
pub struct UsageRecord {
    pub timestamp: String,
    pub session_key: String,
    pub model: String,
    pub provider: String,
    pub agent: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cached_input_tokens: u64,
    pub call_type: String,
}

/// Filter parameters for usage queries.
#[derive(Debug, Default)]
pub struct UsageFilter {
    pub from: Option<String>,
    pub to: Option<String>,
    pub model: Option<String>,
    pub provider: Option<String>,
    pub agent: Option<String>,
    pub session: Option<String>,
    pub group_by: Option<String>,
}

/// Aggregated usage summary row.
#[derive(Debug, Serialize)]
pub struct UsageSummary {
    pub group_key: String,
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cached_input_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_cost: Option<f64>,
}

/// Per-day usage breakdown.
#[derive(Debug, Serialize)]
pub struct DailyUsage {
    pub date: String,
    pub calls: u64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub total_tokens: u64,
    pub cached_input_tokens: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub estimated_cost: Option<f64>,
}

/// Tracks LLM API usage in a SQLite database.
pub struct UsageTracker {
    conn: Mutex<Connection>,
}

impl UsageTracker {
    /// Open or create the usage database.
    pub fn new(db_path: &Path) -> Result<Self> {
        if let Some(parent) = db_path.parent() {
            std::fs::create_dir_all(parent)?;
        }

        let conn = Connection::open(db_path)?;
        conn.execute_batch("PRAGMA journal_mode=WAL;")?;

        conn.execute_batch(
            "CREATE TABLE IF NOT EXISTS usage (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                timestamp TEXT NOT NULL,
                session_key TEXT NOT NULL,
                model TEXT NOT NULL,
                provider TEXT NOT NULL,
                agent TEXT NOT NULL DEFAULT 'default',
                input_tokens INTEGER NOT NULL,
                output_tokens INTEGER NOT NULL,
                total_tokens INTEGER NOT NULL,
                cached_input_tokens INTEGER NOT NULL DEFAULT 0,
                call_type TEXT NOT NULL DEFAULT 'chat'
            );
            CREATE INDEX IF NOT EXISTS idx_usage_timestamp ON usage(timestamp);
            CREATE INDEX IF NOT EXISTS idx_usage_session ON usage(session_key);
            CREATE INDEX IF NOT EXISTS idx_usage_model ON usage(model);
            CREATE INDEX IF NOT EXISTS idx_usage_agent ON usage(agent);",
        )?;

        Ok(Self {
            conn: Mutex::new(conn),
        })
    }

    fn lock_conn(&self) -> Result<std::sync::MutexGuard<'_, Connection>> {
        self.conn
            .lock()
            .map_err(|e| anyhow::anyhow!("Lock poisoned: {e}"))
    }

    /// Record a single LLM API call.
    pub fn record(&self, rec: &UsageRecord) {
        let conn = match self.lock_conn() {
            Ok(c) => c,
            Err(e) => {
                tracing::warn!("Usage tracking failed (lock): {e}");
                return;
            }
        };
        if let Err(e) = conn.execute(
            "INSERT INTO usage (timestamp, session_key, model, provider, agent, input_tokens, output_tokens, total_tokens, cached_input_tokens, call_type)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10)",
            rusqlite::params![
                rec.timestamp,
                rec.session_key,
                rec.model,
                rec.provider,
                rec.agent,
                rec.input_tokens as i64,
                rec.output_tokens as i64,
                rec.total_tokens as i64,
                rec.cached_input_tokens as i64,
                rec.call_type,
            ],
        ) {
            tracing::warn!("Usage tracking failed (insert): {e}");
        }
    }

    /// Query aggregated usage, grouped by a specified column.
    ///
    /// Valid `group_by` values: "model", "provider", "agent", "session", "day", "call_type".
    /// Falls back to "model" if invalid.
    pub fn query_summary(&self, filter: &UsageFilter) -> Result<Vec<UsageSummary>> {
        let conn = self.lock_conn()?;

        let group_col = match filter.group_by.as_deref() {
            Some("model") => "model",
            Some("provider") => "provider",
            Some("agent") => "agent",
            Some("session") => "session_key",
            Some("call_type") => "call_type",
            Some("day") => "date(timestamp)",
            _ => "model",
        };

        let (where_clause, params) = build_where_clause(filter);

        let sql = format!(
            "SELECT {group_col} AS group_key,
                    COUNT(*) AS calls,
                    SUM(input_tokens) AS input_tokens,
                    SUM(output_tokens) AS output_tokens,
                    SUM(total_tokens) AS total_tokens,
                    SUM(cached_input_tokens) AS cached_input_tokens
             FROM usage
             {where_clause}
             GROUP BY group_key
             ORDER BY total_tokens DESC"
        );

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |row| {
                Ok(UsageSummary {
                    group_key: row.get(0)?,
                    calls: row.get::<_, i64>(1)? as u64,
                    input_tokens: row.get::<_, i64>(2)? as u64,
                    output_tokens: row.get::<_, i64>(3)? as u64,
                    total_tokens: row.get::<_, i64>(4)? as u64,
                    cached_input_tokens: row.get::<_, i64>(5)? as u64,
                    estimated_cost: None,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(rows)
    }

    /// Query per-day usage totals.
    pub fn query_daily(&self, filter: &UsageFilter) -> Result<Vec<DailyUsage>> {
        let conn = self.lock_conn()?;

        let (where_clause, params) = build_where_clause(filter);

        let sql = format!(
            "SELECT date(timestamp) AS day,
                    COUNT(*) AS calls,
                    SUM(input_tokens) AS input_tokens,
                    SUM(output_tokens) AS output_tokens,
                    SUM(total_tokens) AS total_tokens,
                    SUM(cached_input_tokens) AS cached_input_tokens
             FROM usage
             {where_clause}
             GROUP BY day
             ORDER BY day DESC"
        );

        let mut stmt = conn.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |row| {
                Ok(DailyUsage {
                    date: row.get(0)?,
                    calls: row.get::<_, i64>(1)? as u64,
                    input_tokens: row.get::<_, i64>(2)? as u64,
                    output_tokens: row.get::<_, i64>(3)? as u64,
                    total_tokens: row.get::<_, i64>(4)? as u64,
                    cached_input_tokens: row.get::<_, i64>(5)? as u64,
                    estimated_cost: None,
                })
            })?
            .filter_map(|r| r.ok())
            .collect();

        Ok(rows)
    }

    /// Query aggregated usage with estimated costs applied from pricing config.
    ///
    /// When `group_by` is "model", cost is calculated directly per row.
    /// For other groupings, a secondary per-model query is run to calculate
    /// accurate costs, then results are re-aggregated.
    pub fn query_summary_with_cost(
        &self,
        filter: &UsageFilter,
        pricing: &HashMap<String, ModelPricing>,
    ) -> Result<Vec<UsageSummary>> {
        if pricing.is_empty() {
            // No pricing configured — return plain results with None costs
            return self.query_summary(filter);
        }

        let group_by = filter.group_by.as_deref().unwrap_or("model");

        if group_by == "model" {
            // Direct: each row is already per-model, apply pricing inline
            let mut rows = self.query_summary(filter)?;
            for row in &mut rows {
                row.estimated_cost = pricing.get(&row.group_key).map(|p| {
                    calculate_cost(
                        row.input_tokens,
                        row.output_tokens,
                        row.cached_input_tokens,
                        p,
                    )
                });
            }
            return Ok(rows);
        }

        // For non-model groupings, we need per-model token counts to apply
        // pricing accurately. Query per-(group_key, model) from SQL, apply
        // pricing in Rust, then re-aggregate by group_key.
        let conn = self.lock_conn()?;

        let group_col = match group_by {
            "provider" => "provider",
            "agent" => "agent",
            "session" => "session_key",
            "call_type" => "call_type",
            "day" => "date(timestamp)",
            _ => "model",
        };

        let (where_clause, params) = build_where_clause(filter);

        let sql = format!(
            "SELECT {group_col} AS group_key, model,
                    COUNT(*) AS calls,
                    SUM(input_tokens) AS input_tokens,
                    SUM(output_tokens) AS output_tokens,
                    SUM(total_tokens) AS total_tokens,
                    SUM(cached_input_tokens) AS cached_input_tokens
             FROM usage
             {where_clause}
             GROUP BY group_key, model
             ORDER BY total_tokens DESC"
        );

        let mut stmt = conn.prepare(&sql)?;
        let detail_rows: Vec<(String, String, u64, u64, u64, u64, u64)> = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? as u64,
                    row.get::<_, i64>(3)? as u64,
                    row.get::<_, i64>(4)? as u64,
                    row.get::<_, i64>(5)? as u64,
                    row.get::<_, i64>(6)? as u64,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        // Re-aggregate by group_key, summing costs across models
        let mut agg: HashMap<String, UsageSummary> = HashMap::new();
        for (gk, model, calls, inp, out, total, cached) in detail_rows {
            let entry = agg.entry(gk.clone()).or_insert_with(|| UsageSummary {
                group_key: gk,
                calls: 0,
                input_tokens: 0,
                output_tokens: 0,
                total_tokens: 0,
                cached_input_tokens: 0,
                estimated_cost: Some(0.0),
            });
            entry.calls += calls;
            entry.input_tokens += inp;
            entry.output_tokens += out;
            entry.total_tokens += total;
            entry.cached_input_tokens += cached;
            if let Some(p) = pricing.get(&model) {
                *entry.estimated_cost.as_mut().unwrap() += calculate_cost(inp, out, cached, p);
            }
        }

        let mut results: Vec<UsageSummary> = agg.into_values().collect();
        results.sort_by(|a, b| b.total_tokens.cmp(&a.total_tokens));
        Ok(results)
    }

    /// Query per-day usage with estimated costs.
    pub fn query_daily_with_cost(
        &self,
        filter: &UsageFilter,
        pricing: &HashMap<String, ModelPricing>,
    ) -> Result<Vec<DailyUsage>> {
        if pricing.is_empty() {
            return self.query_daily(filter);
        }

        let conn = self.lock_conn()?;
        let (where_clause, params) = build_where_clause(filter);

        // Query per-(day, model) so we can apply per-model pricing
        let sql = format!(
            "SELECT date(timestamp) AS day, model,
                    COUNT(*) AS calls,
                    SUM(input_tokens) AS input_tokens,
                    SUM(output_tokens) AS output_tokens,
                    SUM(total_tokens) AS total_tokens,
                    SUM(cached_input_tokens) AS cached_input_tokens
             FROM usage
             {where_clause}
             GROUP BY day, model
             ORDER BY day DESC"
        );

        let mut stmt = conn.prepare(&sql)?;
        let detail_rows: Vec<(String, String, u64, u64, u64, u64, u64)> = stmt
            .query_map(rusqlite::params_from_iter(params.iter()), |row| {
                Ok((
                    row.get::<_, String>(0)?,
                    row.get::<_, String>(1)?,
                    row.get::<_, i64>(2)? as u64,
                    row.get::<_, i64>(3)? as u64,
                    row.get::<_, i64>(4)? as u64,
                    row.get::<_, i64>(5)? as u64,
                    row.get::<_, i64>(6)? as u64,
                ))
            })?
            .filter_map(|r| r.ok())
            .collect();

        let mut agg: HashMap<String, DailyUsage> = HashMap::new();
        for (day, model, calls, inp, out, total, cached) in detail_rows {
            let entry = agg.entry(day.clone()).or_insert_with(|| DailyUsage {
                date: day,
                calls: 0,
                input_tokens: 0,
                output_tokens: 0,
                total_tokens: 0,
                cached_input_tokens: 0,
                estimated_cost: Some(0.0),
            });
            entry.calls += calls;
            entry.input_tokens += inp;
            entry.output_tokens += out;
            entry.total_tokens += total;
            entry.cached_input_tokens += cached;
            if let Some(p) = pricing.get(&model) {
                *entry.estimated_cost.as_mut().unwrap() += calculate_cost(inp, out, cached, p);
            }
        }

        let mut results: Vec<DailyUsage> = agg.into_values().collect();
        results.sort_by(|a, b| b.date.cmp(&a.date));
        Ok(results)
    }

    /// Get distinct values for a column (for populating filter dropdowns).
    pub fn distinct_values(&self, column: &str) -> Result<Vec<String>> {
        let col = match column {
            "model" | "provider" | "agent" | "call_type" => column,
            "session" => "session_key",
            _ => return Ok(Vec::new()),
        };

        let conn = self.lock_conn()?;
        let sql = format!("SELECT DISTINCT {col} FROM usage ORDER BY {col}");
        let mut stmt = conn.prepare(&sql)?;
        let values = stmt
            .query_map([], |row| row.get(0))?
            .filter_map(|r| r.ok())
            .collect();
        Ok(values)
    }
}

/// Build a WHERE clause from filter parameters.
/// Returns (clause_string, param_values).
fn build_where_clause(filter: &UsageFilter) -> (String, Vec<String>) {
    let mut conditions = Vec::new();
    let mut params = Vec::new();

    if let Some(ref from) = filter.from {
        params.push(from.clone());
        conditions.push(format!("timestamp >= ?{}", params.len()));
    }
    if let Some(ref to) = filter.to {
        params.push(to.clone());
        conditions.push(format!("timestamp <= ?{}", params.len()));
    }
    if let Some(ref model) = filter.model {
        params.push(model.clone());
        conditions.push(format!("model = ?{}", params.len()));
    }
    if let Some(ref provider) = filter.provider {
        params.push(provider.clone());
        conditions.push(format!("provider = ?{}", params.len()));
    }
    if let Some(ref agent) = filter.agent {
        params.push(agent.clone());
        conditions.push(format!("agent = ?{}", params.len()));
    }
    if let Some(ref session) = filter.session {
        params.push(session.clone());
        conditions.push(format!("session_key = ?{}", params.len()));
    }

    let clause = if conditions.is_empty() {
        String::new()
    } else {
        format!("WHERE {}", conditions.join(" AND "))
    };

    (clause, params)
}

/// Calculate the estimated cost for a set of token counts given a pricing config.
pub fn calculate_cost(
    input_tokens: u64,
    output_tokens: u64,
    cached_input_tokens: u64,
    pricing: &ModelPricing,
) -> f64 {
    // rig-core includes cached tokens in input_tokens, so subtract them
    // to avoid double-counting: uncached tokens at full rate, cached at cached rate.
    let uncached = input_tokens.saturating_sub(cached_input_tokens);
    let input_cost = (uncached as f64 / 1_000_000.0) * pricing.input;
    let output_cost = (output_tokens as f64 / 1_000_000.0) * pricing.output;
    let cached_rate = if pricing.cached_input > 0.0 {
        pricing.cached_input
    } else {
        pricing.input
    };
    let cached_cost = (cached_input_tokens as f64 / 1_000_000.0) * cached_rate;
    input_cost + output_cost + cached_cost
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker_in_memory() -> UsageTracker {
        let dir = tempfile::tempdir().unwrap();
        UsageTracker::new(&dir.path().join("test_usage.sqlite")).unwrap()
    }

    fn sample_record(model: &str, provider: &str, agent: &str, tokens: u64) -> UsageRecord {
        UsageRecord {
            timestamp: "2026-02-20T12:00:00Z".to_string(),
            session_key: "web:abc-123".to_string(),
            model: model.to_string(),
            provider: provider.to_string(),
            agent: agent.to_string(),
            input_tokens: tokens,
            output_tokens: tokens / 2,
            total_tokens: tokens + tokens / 2,
            cached_input_tokens: 0,
            call_type: "chat".to_string(),
        }
    }

    #[test]
    fn test_record_and_query_summary() {
        let tracker = tracker_in_memory();
        tracker.record(&sample_record("gpt-4", "openai", "default", 100));
        tracker.record(&sample_record("gpt-4", "openai", "default", 200));
        tracker.record(&sample_record("claude-3", "anthropic", "coder", 300));

        let results = tracker
            .query_summary(&UsageFilter {
                group_by: Some("model".to_string()),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(results.len(), 2);
        // Ordered by total_tokens DESC
        assert_eq!(results[0].group_key, "claude-3");
        assert_eq!(results[0].calls, 1);
        assert_eq!(results[0].input_tokens, 300);

        assert_eq!(results[1].group_key, "gpt-4");
        assert_eq!(results[1].calls, 2);
        assert_eq!(results[1].input_tokens, 300); // 100 + 200
    }

    #[test]
    fn test_filter_by_model() {
        let tracker = tracker_in_memory();
        tracker.record(&sample_record("gpt-4", "openai", "default", 100));
        tracker.record(&sample_record("claude-3", "anthropic", "default", 200));

        let results = tracker
            .query_summary(&UsageFilter {
                model: Some("gpt-4".to_string()),
                group_by: Some("model".to_string()),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].group_key, "gpt-4");
    }

    #[test]
    fn test_filter_by_agent() {
        let tracker = tracker_in_memory();
        tracker.record(&sample_record("gpt-4", "openai", "default", 100));
        tracker.record(&sample_record("gpt-4", "openai", "coder", 200));
        tracker.record(&sample_record("gpt-4", "openai", "coder", 300));

        let results = tracker
            .query_summary(&UsageFilter {
                agent: Some("coder".to_string()),
                group_by: Some("agent".to_string()),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].group_key, "coder");
        assert_eq!(results[0].calls, 2);
        assert_eq!(results[0].input_tokens, 500);
    }

    #[test]
    fn test_group_by_agent() {
        let tracker = tracker_in_memory();
        tracker.record(&sample_record("gpt-4", "openai", "default", 100));
        tracker.record(&sample_record("gpt-4", "openai", "coder", 200));
        tracker.record(&sample_record("claude-3", "anthropic", "subagent:abc", 300));

        let results = tracker
            .query_summary(&UsageFilter {
                group_by: Some("agent".to_string()),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(results.len(), 3);
        let agents: Vec<&str> = results.iter().map(|r| r.group_key.as_str()).collect();
        assert!(agents.contains(&"default"));
        assert!(agents.contains(&"coder"));
        assert!(agents.contains(&"subagent:abc"));
    }

    #[test]
    fn test_group_by_day() {
        let tracker = tracker_in_memory();

        let mut rec1 = sample_record("gpt-4", "openai", "default", 100);
        rec1.timestamp = "2026-02-19T10:00:00Z".to_string();
        tracker.record(&rec1);

        let mut rec2 = sample_record("gpt-4", "openai", "default", 200);
        rec2.timestamp = "2026-02-20T10:00:00Z".to_string();
        tracker.record(&rec2);

        let mut rec3 = sample_record("gpt-4", "openai", "default", 300);
        rec3.timestamp = "2026-02-20T15:00:00Z".to_string();
        tracker.record(&rec3);

        let results = tracker.query_daily(&UsageFilter::default()).unwrap();

        assert_eq!(results.len(), 2);
        // Ordered by day DESC
        assert_eq!(results[0].date, "2026-02-20");
        assert_eq!(results[0].calls, 2);
        assert_eq!(results[0].input_tokens, 500);

        assert_eq!(results[1].date, "2026-02-19");
        assert_eq!(results[1].calls, 1);
        assert_eq!(results[1].input_tokens, 100);
    }

    #[test]
    fn test_filter_by_date_range() {
        let tracker = tracker_in_memory();

        let mut rec1 = sample_record("gpt-4", "openai", "default", 100);
        rec1.timestamp = "2026-02-18T10:00:00Z".to_string();
        tracker.record(&rec1);

        let mut rec2 = sample_record("gpt-4", "openai", "default", 200);
        rec2.timestamp = "2026-02-20T10:00:00Z".to_string();
        tracker.record(&rec2);

        let results = tracker
            .query_summary(&UsageFilter {
                from: Some("2026-02-19T00:00:00Z".to_string()),
                group_by: Some("model".to_string()),
                ..Default::default()
            })
            .unwrap();

        assert_eq!(results.len(), 1);
        assert_eq!(results[0].input_tokens, 200);
    }

    #[test]
    fn test_empty_db_returns_empty() {
        let tracker = tracker_in_memory();

        let summary = tracker.query_summary(&UsageFilter::default()).unwrap();
        assert!(summary.is_empty());

        let daily = tracker.query_daily(&UsageFilter::default()).unwrap();
        assert!(daily.is_empty());
    }

    #[test]
    fn test_distinct_values() {
        let tracker = tracker_in_memory();
        tracker.record(&sample_record("gpt-4", "openai", "default", 100));
        tracker.record(&sample_record("claude-3", "anthropic", "coder", 200));
        tracker.record(&sample_record("gpt-4", "openai", "default", 300));

        let models = tracker.distinct_values("model").unwrap();
        assert_eq!(models, vec!["claude-3", "gpt-4"]);

        let providers = tracker.distinct_values("provider").unwrap();
        assert_eq!(providers, vec!["anthropic", "openai"]);

        let agents = tracker.distinct_values("agent").unwrap();
        assert_eq!(agents, vec!["coder", "default"]);
    }

    #[test]
    fn test_distinct_values_invalid_column() {
        let tracker = tracker_in_memory();
        let result = tracker.distinct_values("nonexistent").unwrap();
        assert!(result.is_empty());
    }

    fn test_pricing() -> HashMap<String, ModelPricing> {
        let mut pricing = HashMap::new();
        pricing.insert(
            "gpt-4".to_string(),
            ModelPricing {
                input: 30.0,  // $30/1M input
                output: 60.0, // $60/1M output
                cached_input: 15.0,
            },
        );
        pricing.insert(
            "claude-3".to_string(),
            ModelPricing {
                input: 3.0,
                output: 15.0,
                cached_input: 0.0, // falls back to input rate
            },
        );
        pricing
    }

    #[test]
    fn test_calculate_cost() {
        let pricing = ModelPricing {
            input: 3.0,
            output: 15.0,
            cached_input: 0.30,
        };
        // input_tokens includes cached, so: 800K uncached × $3 = $2.40,
        // 500K output × $15 = $7.50, 200K cached × $0.30 = $0.06
        let cost = calculate_cost(1_000_000, 500_000, 200_000, &pricing);
        assert!((cost - 9.96).abs() < 0.001);
    }

    #[test]
    fn test_calculate_cost_cached_fallback_to_input() {
        let pricing = ModelPricing {
            input: 3.0,
            output: 15.0,
            cached_input: 0.0, // should use input rate
        };
        // input_tokens includes cached, so: 0 uncached × $3 = $0,
        // 0 output, 1M cached at input rate = $3
        let cost = calculate_cost(1_000_000, 0, 1_000_000, &pricing);
        assert!((cost - 3.0).abs() < 0.001);
    }

    #[test]
    fn test_summary_with_cost_by_model() {
        let tracker = tracker_in_memory();
        tracker.record(&sample_record("gpt-4", "openai", "default", 1000));
        tracker.record(&sample_record("claude-3", "anthropic", "default", 2000));

        let pricing = test_pricing();
        let results = tracker
            .query_summary_with_cost(
                &UsageFilter {
                    group_by: Some("model".to_string()),
                    ..Default::default()
                },
                &pricing,
            )
            .unwrap();

        assert_eq!(results.len(), 2);
        // Both should have estimated_cost set
        for row in &results {
            assert!(row.estimated_cost.is_some());
        }
    }

    #[test]
    fn test_summary_with_cost_by_agent() {
        let tracker = tracker_in_memory();
        tracker.record(&sample_record("gpt-4", "openai", "default", 1000));
        tracker.record(&sample_record("claude-3", "anthropic", "default", 2000));
        tracker.record(&sample_record("gpt-4", "openai", "coder", 500));

        let pricing = test_pricing();
        let results = tracker
            .query_summary_with_cost(
                &UsageFilter {
                    group_by: Some("agent".to_string()),
                    ..Default::default()
                },
                &pricing,
            )
            .unwrap();

        assert_eq!(results.len(), 2);
        // "default" agent uses both gpt-4 and claude-3, costs should be summed
        let default_row = results.iter().find(|r| r.group_key == "default").unwrap();
        assert!(default_row.estimated_cost.unwrap() > 0.0);
        assert_eq!(default_row.calls, 2);
    }

    #[test]
    fn test_summary_with_cost_empty_pricing() {
        let tracker = tracker_in_memory();
        tracker.record(&sample_record("gpt-4", "openai", "default", 1000));

        let empty: HashMap<String, ModelPricing> = HashMap::new();
        let results = tracker
            .query_summary_with_cost(
                &UsageFilter {
                    group_by: Some("model".to_string()),
                    ..Default::default()
                },
                &empty,
            )
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].estimated_cost.is_none());
    }

    #[test]
    fn test_summary_with_cost_unpriced_model() {
        let tracker = tracker_in_memory();
        tracker.record(&sample_record("llama-3", "ollama", "default", 1000));

        let pricing = test_pricing(); // doesn't include llama-3
        let results = tracker
            .query_summary_with_cost(
                &UsageFilter {
                    group_by: Some("model".to_string()),
                    ..Default::default()
                },
                &pricing,
            )
            .unwrap();

        assert_eq!(results.len(), 1);
        assert!(results[0].estimated_cost.is_none()); // no pricing for this model
    }

    #[test]
    fn test_daily_with_cost() {
        let tracker = tracker_in_memory();

        let mut rec1 = sample_record("gpt-4", "openai", "default", 1000);
        rec1.timestamp = "2026-02-19T10:00:00Z".to_string();
        tracker.record(&rec1);

        let mut rec2 = sample_record("claude-3", "anthropic", "default", 2000);
        rec2.timestamp = "2026-02-20T10:00:00Z".to_string();
        tracker.record(&rec2);

        let pricing = test_pricing();
        let results = tracker
            .query_daily_with_cost(&UsageFilter::default(), &pricing)
            .unwrap();

        assert_eq!(results.len(), 2);
        // Both days should have costs
        assert!(results[0].estimated_cost.unwrap() > 0.0);
        assert!(results[1].estimated_cost.unwrap() > 0.0);
        // Ordered by date DESC
        assert_eq!(results[0].date, "2026-02-20");
        assert_eq!(results[1].date, "2026-02-19");
    }
}
