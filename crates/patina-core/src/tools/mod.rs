pub mod cron;
pub mod filesystem;
pub mod memory_search;
pub mod message;
pub mod shell;
pub mod spawn;
pub mod task;
pub mod web;

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;

/// Trait for tools callable by the LLM agent.
#[async_trait]
pub trait Tool: Send + Sync {
    fn name(&self) -> &str;
    fn description(&self) -> &str;
    fn parameters_schema(&self) -> serde_json::Value;
    async fn execute(&self, params: serde_json::Value) -> Result<String>;
}

/// Registry of available tools.
pub struct ToolRegistry {
    tools: HashMap<String, Box<dyn Tool>>,
}

impl ToolRegistry {
    pub fn new() -> Self {
        Self {
            tools: HashMap::new(),
        }
    }

    pub fn register(&mut self, tool: Box<dyn Tool>) {
        self.tools.insert(tool.name().to_string(), tool);
    }

    pub fn get(&self, name: &str) -> Option<&dyn Tool> {
        self.tools.get(name).map(|t| t.as_ref())
    }

    pub fn list(&self) -> Vec<&dyn Tool> {
        self.tools.values().map(|t| t.as_ref()).collect()
    }

    /// Get tool definitions in OpenAI function-calling format.
    pub fn get_definitions(&self) -> Vec<serde_json::Value> {
        self.tools
            .values()
            .map(|t| {
                serde_json::json!({
                    "type": "function",
                    "function": {
                        "name": t.name(),
                        "description": t.description(),
                        "parameters": t.parameters_schema(),
                    }
                })
            })
            .collect()
    }

    pub async fn execute(&self, name: &str, params: serde_json::Value) -> Result<String> {
        match self.tools.get(name) {
            Some(tool) => {
                let errors = validate_params(&params, &tool.parameters_schema());
                if !errors.is_empty() {
                    return Ok(format!(
                        "Error: Invalid parameters for tool '{}': {}",
                        name,
                        errors.join("; ")
                    ));
                }
                tool.execute(params).await
            }
            None => anyhow::bail!("unknown tool: {name}"),
        }
    }
}

impl Default for ToolRegistry {
    fn default() -> Self {
        Self::new()
    }
}

/// Validate tool parameters against a JSON schema.
/// Returns a list of validation error strings (empty if valid).
fn validate_params(params: &serde_json::Value, schema: &serde_json::Value) -> Vec<String> {
    let mut errors = Vec::new();
    validate_value(params, schema, "", &mut errors);
    errors
}

fn validate_value(
    val: &serde_json::Value,
    schema: &serde_json::Value,
    path: &str,
    errors: &mut Vec<String>,
) {
    let display_path = if path.is_empty() { "root" } else { path };

    // Check type
    if let Some(expected_type) = schema.get("type").and_then(|t| t.as_str()) {
        let type_ok = match expected_type {
            "object" => val.is_object(),
            "array" => val.is_array(),
            "string" => val.is_string(),
            "integer" => val.is_i64() || val.is_u64(),
            "number" => val.is_number(),
            "boolean" => val.is_boolean(),
            "null" => val.is_null(),
            _ => true,
        };
        if !type_ok {
            errors.push(format!("{display_path}: expected type '{expected_type}'"));
            return;
        }
    }

    // Check enum
    if let Some(allowed) = schema.get("enum").and_then(|e| e.as_array()) {
        if !allowed.contains(val) {
            errors.push(format!("{display_path}: value not in allowed enum"));
        }
    }

    // Numeric constraints
    if let Some(n) = val.as_f64() {
        if let Some(min) = schema.get("minimum").and_then(|m| m.as_f64()) {
            if n < min {
                errors.push(format!("{display_path}: value {n} < minimum {min}"));
            }
        }
        if let Some(max) = schema.get("maximum").and_then(|m| m.as_f64()) {
            if n > max {
                errors.push(format!("{display_path}: value {n} > maximum {max}"));
            }
        }
    }

    // String constraints
    if let Some(s) = val.as_str() {
        if let Some(min_len) = schema.get("minLength").and_then(|m| m.as_u64()) {
            if (s.len() as u64) < min_len {
                errors.push(format!(
                    "{display_path}: string length {} < minLength {min_len}",
                    s.len()
                ));
            }
        }
        if let Some(max_len) = schema.get("maxLength").and_then(|m| m.as_u64()) {
            if (s.len() as u64) > max_len {
                errors.push(format!(
                    "{display_path}: string length {} > maxLength {max_len}",
                    s.len()
                ));
            }
        }
    }

    // Object: check required fields and validate properties
    if let Some(obj) = val.as_object() {
        if let Some(required) = schema.get("required").and_then(|r| r.as_array()) {
            for req in required {
                if let Some(field) = req.as_str() {
                    if !obj.contains_key(field) {
                        let field_path = if path.is_empty() {
                            field.to_string()
                        } else {
                            format!("{path}.{field}")
                        };
                        errors.push(format!("{field_path}: required field missing"));
                    }
                }
            }
        }
        if let Some(props) = schema.get("properties").and_then(|p| p.as_object()) {
            for (key, prop_schema) in props {
                if let Some(prop_val) = obj.get(key) {
                    let prop_path = if path.is_empty() {
                        key.clone()
                    } else {
                        format!("{path}.{key}")
                    };
                    validate_value(prop_val, prop_schema, &prop_path, errors);
                }
            }
        }
    }

    // Array: validate items
    if let Some(arr) = val.as_array() {
        if let Some(items_schema) = schema.get("items") {
            for (i, item) in arr.iter().enumerate() {
                let item_path = format!("{display_path}[{i}]");
                validate_value(item, items_schema, &item_path, errors);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_valid_params() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"},
                "count": {"type": "integer", "minimum": 1, "maximum": 10}
            },
            "required": ["query"]
        });
        let params = serde_json::json!({"query": "test", "count": 5});
        assert!(validate_params(&params, &schema).is_empty());
    }

    #[test]
    fn test_missing_required() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"}
            },
            "required": ["query"]
        });
        let params = serde_json::json!({});
        let errors = validate_params(&params, &schema);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("required field missing"));
    }

    #[test]
    fn test_wrong_type() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "count": {"type": "integer"}
            },
            "required": ["count"]
        });
        let params = serde_json::json!({"count": "not_a_number"});
        let errors = validate_params(&params, &schema);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("expected type 'integer'"));
    }

    #[test]
    fn test_numeric_range() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "count": {"type": "integer", "minimum": 1, "maximum": 10}
            },
            "required": ["count"]
        });
        let params = serde_json::json!({"count": 15});
        let errors = validate_params(&params, &schema);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("maximum"));
    }

    #[test]
    fn test_enum_validation() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "mode": {"type": "string", "enum": ["read", "write"]}
            },
            "required": ["mode"]
        });
        let params = serde_json::json!({"mode": "delete"});
        let errors = validate_params(&params, &schema);
        assert_eq!(errors.len(), 1);
        assert!(errors[0].contains("enum"));
    }

    #[test]
    fn test_extra_fields_ignored() {
        let schema = serde_json::json!({
            "type": "object",
            "properties": {
                "query": {"type": "string"}
            },
            "required": ["query"]
        });
        let params = serde_json::json!({"query": "test", "extra": "ignored"});
        assert!(validate_params(&params, &schema).is_empty());
    }
}
