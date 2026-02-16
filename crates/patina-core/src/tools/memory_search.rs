use std::sync::Arc;

use anyhow::Result;
use async_trait::async_trait;

use super::Tool;
use crate::agent::memory_index::MemoryIndex;

/// Tool that searches memory and history files using FTS5 full-text search.
pub struct MemorySearchTool {
    index: Arc<MemoryIndex>,
}

impl MemorySearchTool {
    pub fn new(index: Arc<MemoryIndex>) -> Self {
        Self { index }
    }
}

#[async_trait]
impl Tool for MemorySearchTool {
    fn name(&self) -> &str {
        "memory_search"
    }

    fn description(&self) -> &str {
        "Search memory and history files using full-text search. Returns relevant passages with file locations."
    }

    fn parameters_schema(&self) -> serde_json::Value {
        serde_json::json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search query (keywords to find in memory/history files)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of results to return",
                    "minimum": 1,
                    "maximum": 20
                }
            },
            "required": ["query"]
        })
    }

    async fn execute(&self, params: serde_json::Value) -> Result<String> {
        let query = params
            .get("query")
            .and_then(|q| q.as_str())
            .unwrap_or("")
            .to_string();

        if query.trim().is_empty() {
            return Ok("Error: query is required".into());
        }

        let limit = params
            .get("limit")
            .and_then(|l| l.as_u64())
            .map(|l| l.clamp(1, 20) as usize)
            .unwrap_or(5);

        let results = self.index.search(&query, limit)?;

        if results.is_empty() {
            return Ok("No results found.".into());
        }

        let mut output = String::new();
        for (i, result) in results.iter().enumerate() {
            if i > 0 {
                output.push_str("\n---\n");
            }
            output.push_str(&format!(
                "## Result {} (score: {:.2})\n**File:** {} (lines {}-{})\n\n{}\n",
                i + 1,
                result.score,
                result.path,
                result.start_line,
                result.end_line,
                result.content,
            ));
        }

        Ok(output)
    }
}
