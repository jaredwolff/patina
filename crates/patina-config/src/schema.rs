use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub agents: AgentsConfig,
    pub channels: ChannelsConfig,
    pub providers: ProvidersConfig,
    pub tools: ToolsConfig,
    pub gateway: GatewayConfig,
    pub heartbeat: HeartbeatConfig,
    pub transcription: TranscriptionConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentsConfig {
    pub defaults: AgentDefaults,
    /// Named model tiers. Must contain at least a "default" entry.
    /// Example tiers: "default", "coding", "consolidation".
    pub models: HashMap<String, ModelRef>,
    /// Per-model pricing in $/1M tokens, keyed by model name.
    pub pricing: HashMap<String, ModelPricing>,
}

/// Reference to a provider + model combination for a named tier.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelRef {
    pub provider: String,
    pub model: String,
}

/// Per-model pricing in dollars per 1M tokens.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelPricing {
    pub input: f64,
    pub output: f64,
    #[serde(default)]
    pub cached_input: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentDefaults {
    pub workspace: String,
    /// Deprecated: use agents.models instead.
    #[serde(default)]
    pub provider: String,
    /// Deprecated: use agents.models instead.
    #[serde(default)]
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub max_tool_iterations: u32,
    pub memory_window: usize,
}

impl Default for AgentDefaults {
    fn default() -> Self {
        Self {
            workspace: "~/.patina/workspace".into(),
            provider: String::new(),
            model: String::new(),
            max_tokens: 8192,
            temperature: 0.7,
            max_tool_iterations: 20,
            memory_window: 30,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ChannelsConfig {
    pub telegram: TelegramConfig,
    pub slack: SlackConfig,
    pub web: WebConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TelegramConfig {
    pub enabled: bool,
    pub token: String,
    pub allow_from: Vec<String>,
    pub proxy: Option<String>,
    /// Update mode: "polling" (default) or "webhook".
    pub mode: TelegramMode,
    /// Public HTTPS URL for webhook mode (e.g. "https://example.com/webhook/telegram").
    /// Telegram only supports ports 443, 80, 88, and 8443.
    pub webhook_url: Option<String>,
    /// Local address to bind the webhook listener on (default: "0.0.0.0").
    pub webhook_listen: Option<String>,
    /// Local port to bind the webhook listener on (default: 8443).
    pub webhook_port: Option<u16>,
    /// Optional override for channel-specific system prompt rules.
    pub system_prompt_rules: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TelegramMode {
    #[default]
    Polling,
    Webhook,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct SlackConfig {
    pub enabled: bool,
    /// App-level token (xapp-*) for Socket Mode WebSocket connection.
    pub app_token: String,
    /// Bot token (xoxb-*) for Web API calls.
    pub bot_token: String,
    pub allow_from: Vec<String>,
    /// Optional override for channel-specific system prompt rules.
    pub system_prompt_rules: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct WebConfig {
    pub enabled: bool,
    /// Optional password for access control. If empty, no auth required.
    pub password: String,
    pub allow_from: Vec<String>,
    /// Optional override for channel-specific system prompt rules.
    pub system_prompt_rules: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ProvidersConfig {
    pub ollama: Option<ProviderConfig>,
    pub anthropic: Option<ProviderConfig>,
    pub openai: Option<ProviderConfig>,
    pub openrouter: Option<ProviderConfig>,
    pub deepseek: Option<ProviderConfig>,
    pub groq: Option<ProviderConfig>,
    pub gemini: Option<ProviderConfig>,
    pub mistral: Option<ProviderConfig>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderConfig {
    pub api_key: Option<String>,
    pub api_base: Option<String>,
    pub extra_headers: Option<std::collections::HashMap<String, String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct GatewayConfig {
    pub host: String,
    pub port: u16,
}

impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            host: "0.0.0.0".into(),
            port: 18790,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ToolsConfig {
    pub restrict_to_workspace: bool,
    pub exec: ExecToolConfig,
    pub web: WebToolsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct WebToolsConfig {
    pub search: WebSearchConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct WebSearchConfig {
    pub api_key: String,
    pub max_results: u32,
}

impl Default for WebSearchConfig {
    fn default() -> Self {
        Self {
            api_key: String::new(),
            max_results: 5,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct ExecToolConfig {
    pub timeout_secs: u64,
}

impl Default for ExecToolConfig {
    fn default() -> Self {
        Self { timeout_secs: 60 }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct HeartbeatConfig {
    pub enabled: bool,
    pub interval_secs: u64,
}

impl Default for HeartbeatConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            interval_secs: 1800, // 30 minutes
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct TranscriptionConfig {
    #[serde(alias = "engine")]
    pub mode: TranscriptionMode,
    /// Path to local model directory.
    /// Default: ~/.patina/models/parakeet-tdt
    #[serde(alias = "model")]
    pub model_path: Option<String>,
    /// GPU execution provider: "cpu", "cuda", "migraphx", "tensorrt".
    /// Default: "cpu"
    pub execution_provider: Option<String>,
    /// Auto-download missing local model files on first use.
    #[serde(default = "default_transcription_auto_download")]
    pub auto_download: bool,
    /// Optional base URL for model files (defaults to HuggingFace ONNX repo).
    pub model_url: Option<String>,
}

fn default_transcription_auto_download() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TranscriptionMode {
    Local,
    Groq,
    #[default]
    Auto,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn telegram_mode_defaults_to_polling() {
        let cfg: Config = serde_json::from_value(serde_json::json!({})).unwrap();
        assert_eq!(cfg.channels.telegram.mode, TelegramMode::Polling);
    }

    #[test]
    fn telegram_webhook_mode_parsed() {
        let cfg: Config = serde_json::from_value(serde_json::json!({
            "channels": {
                "telegram": {
                    "mode": "webhook",
                    "webhookUrl": "https://example.com/webhook/telegram",
                    "webhookListen": "127.0.0.1",
                    "webhookPort": 443
                }
            }
        }))
        .unwrap();
        assert_eq!(cfg.channels.telegram.mode, TelegramMode::Webhook);
        assert_eq!(
            cfg.channels.telegram.webhook_url.as_deref(),
            Some("https://example.com/webhook/telegram")
        );
        assert_eq!(
            cfg.channels.telegram.webhook_listen.as_deref(),
            Some("127.0.0.1")
        );
        assert_eq!(cfg.channels.telegram.webhook_port, Some(443));
    }

    #[test]
    fn telegram_webhook_fields_optional() {
        let cfg: Config = serde_json::from_value(serde_json::json!({
            "channels": {
                "telegram": {
                    "mode": "webhook"
                }
            }
        }))
        .unwrap();
        assert_eq!(cfg.channels.telegram.mode, TelegramMode::Webhook);
        assert!(cfg.channels.telegram.webhook_url.is_none());
        assert!(cfg.channels.telegram.webhook_listen.is_none());
        assert!(cfg.channels.telegram.webhook_port.is_none());
    }

    #[test]
    fn slack_config_defaults() {
        let cfg: Config = serde_json::from_value(serde_json::json!({})).unwrap();
        assert!(!cfg.channels.slack.enabled);
        assert!(cfg.channels.slack.app_token.is_empty());
        assert!(cfg.channels.slack.bot_token.is_empty());
        assert!(cfg.channels.slack.allow_from.is_empty());
    }

    #[test]
    fn slack_config_parsed() {
        let cfg: Config = serde_json::from_value(serde_json::json!({
            "channels": {
                "slack": {
                    "enabled": true,
                    "appToken": "xapp-test-token",
                    "botToken": "xoxb-test-token",
                    "allowFrom": ["U123", "alice"]
                }
            }
        }))
        .unwrap();
        assert!(cfg.channels.slack.enabled);
        assert_eq!(cfg.channels.slack.app_token, "xapp-test-token");
        assert_eq!(cfg.channels.slack.bot_token, "xoxb-test-token");
        assert_eq!(cfg.channels.slack.allow_from, vec!["U123", "alice"]);
    }

    #[test]
    fn transcription_alias_engine_and_model_are_supported() {
        let cfg_json = serde_json::json!({
            "transcription": {
                "engine": "local",
                "model": "~/models/parakeet",
                "executionProvider": "cpu"
            }
        });

        let cfg: Config = serde_json::from_value(cfg_json).unwrap();
        assert_eq!(cfg.transcription.mode, TranscriptionMode::Local);
        assert_eq!(
            cfg.transcription.model_path.as_deref(),
            Some("~/models/parakeet")
        );
        assert_eq!(cfg.transcription.execution_provider.as_deref(), Some("cpu"));
    }
}
