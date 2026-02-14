use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct Config {
    pub agents: AgentsConfig,
    pub channels: ChannelsConfig,
    pub providers: ProvidersConfig,
    pub tools: ToolsConfig,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentsConfig {
    pub defaults: AgentDefaults,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct AgentDefaults {
    pub workspace: String,
    pub model: String,
    pub max_tokens: u32,
    pub temperature: f32,
    pub max_tool_iterations: u32,
    pub memory_window: usize,
}

impl Default for AgentDefaults {
    fn default() -> Self {
        Self {
            workspace: "~/.nanobot/workspace".into(),
            model: "llama3.2".into(),
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
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ProviderConfig {
    pub api_key: Option<String>,
    pub api_base: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "camelCase", default)]
pub struct ToolsConfig {
    pub restrict_to_workspace: bool,
    pub exec: ExecToolConfig,
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
