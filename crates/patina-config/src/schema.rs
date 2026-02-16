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
}

/// Reference to a provider + model combination for a named tier.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase")]
pub struct ModelRef {
    pub provider: String,
    pub model: String,
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
            memory_window: 50,
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ChannelsConfig {
    pub telegram: TelegramConfig,
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
