use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use nanobot_channels::manager::ChannelManager;
use nanobot_channels::telegram::TelegramChannel;
use nanobot_config::{find_config_path, load_config, resolve_workspace};
use nanobot_core::agent::subagent::SubagentManager;
use nanobot_core::agent::{AgentLoop, ContextBuilder, ModelOverrides};
use nanobot_core::bus::{MessageBus, OutboundMessage};
use nanobot_core::cron::CronService;
use nanobot_core::session::SessionManager;
use nanobot_core::tools::cron::CronTool;
use nanobot_core::tools::filesystem::{EditFileTool, ListDirTool, ReadFileTool, WriteFileTool};
use nanobot_core::tools::message::MessageTool;
use nanobot_core::tools::shell::ExecTool;
use nanobot_core::tools::spawn::SpawnTool;
use nanobot_core::tools::web::{WebFetchTool, WebSearchTool};
use nanobot_core::tools::ToolRegistry;
#[allow(deprecated)]
use rig::client::completion::CompletionModelHandle;
use rig::client::{CompletionClient, Nothing};
use rig::providers::{anthropic, deepseek, gemini, groq, ollama, openai, openrouter};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;
use tokio::sync::Mutex;

/// Render markdown text to the terminal using termimad.
fn render_markdown(text: &str) {
    let skin = termimad::MadSkin::default();
    skin.print_text(text);
}

/// Save terminal attributes for later restoration.
#[cfg(unix)]
fn save_terminal_state() -> Option<nix::sys::termios::Termios> {
    nix::sys::termios::tcgetattr(std::io::stdin()).ok()
}

/// Restore previously saved terminal attributes.
#[cfg(unix)]
fn restore_terminal_state(saved: &nix::sys::termios::Termios) {
    let _ = nix::sys::termios::tcsetattr(
        std::io::stdin(),
        nix::sys::termios::SetArg::TCSADRAIN,
        saved,
    );
}

/// Flush any pending input from the terminal.
#[cfg(unix)]
fn flush_pending_input() {
    let _ = nix::sys::termios::tcflush(std::io::stdin(), nix::sys::termios::FlushArg::TCIFLUSH);
}

#[derive(Parser)]
#[command(name = "nanobot", about = "Lightweight AI agent", version)]
struct Cli {
    /// Path to config file
    #[arg(short, long)]
    config: Option<PathBuf>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run interactive CLI chat
    Agent {
        /// Single message mode (non-interactive)
        #[arg(short, long)]
        message: Option<String>,

        /// Session ID for conversation tracking
        #[arg(short, long, default_value = "cli:interactive")]
        session: String,
    },
    /// Start gateway with all enabled channels
    Serve,
    /// Initialize configuration and workspace
    Onboard {
        /// Skip interactive prompts and write defaults
        #[arg(long)]
        non_interactive: bool,
    },
    /// Interrupt an active session run
    Interrupt {
        /// Session key to interrupt (format: channel:chat_id)
        #[arg(short, long, default_value = "cli:interactive")]
        session: String,
    },
    /// Show system status and configuration
    Status,
    /// Manage scheduled cron jobs
    Cron {
        #[command(subcommand)]
        action: CronCommands,
    },
    /// Manage channels
    Channels {
        #[command(subcommand)]
        action: ChannelCommands,
    },
}

#[derive(Subcommand)]
enum CronCommands {
    /// List scheduled jobs
    List {
        /// Include disabled jobs
        #[arg(short, long)]
        all: bool,
    },
    /// Add a new scheduled job
    Add {
        /// Job name
        #[arg(long)]
        name: String,
        /// Message to send when triggered
        #[arg(long)]
        message: String,
        /// Interval in seconds (recurring)
        #[arg(long)]
        every: Option<u64>,
        /// Cron expression (e.g. "0 9 * * *")
        #[arg(long)]
        cron: Option<String>,
        /// One-time execution at ISO datetime (e.g. "2025-06-01T09:00:00Z")
        #[arg(long)]
        at: Option<String>,
        /// Deliver result to a channel
        #[arg(long)]
        deliver: bool,
        /// Target channel for delivery
        #[arg(long)]
        channel: Option<String>,
        /// Target chat_id for delivery
        #[arg(long)]
        to: Option<String>,
    },
    /// Remove a job by ID
    Remove {
        /// Job ID to remove
        job_id: String,
    },
    /// Enable or disable a job
    Enable {
        /// Job ID
        job_id: String,
        /// Disable instead of enable
        #[arg(long)]
        disable: bool,
    },
    /// Manually run a job
    Run {
        /// Job ID to run
        job_id: String,
    },
}

#[derive(Subcommand)]
enum ChannelCommands {
    /// Show channel configuration and status
    Status,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Onboard { non_interactive } => {
            return run_onboard(cli.config, non_interactive);
        }
        Commands::Interrupt { session } => {
            return run_interrupt(&session);
        }
        Commands::Status => {
            let config_path = cli.config.unwrap_or_else(find_config_path);
            return run_status(&config_path);
        }
        Commands::Cron { action } => {
            let config_path = cli.config.unwrap_or_else(find_config_path);
            let config = load_config(&config_path)?;
            return run_cron_command(action, &config).await;
        }
        Commands::Channels { action } => {
            let config_path = cli.config.unwrap_or_else(find_config_path);
            let config = load_config(&config_path)?;
            return run_channel_command(action, &config);
        }
        _ => {}
    }

    // Load config for agent/serve commands
    let config_path = cli.config.unwrap_or_else(find_config_path);
    let config = load_config(&config_path)?;

    // Resolve workspace
    let workspace = resolve_workspace(&config.agents.defaults.workspace);
    std::fs::create_dir_all(&workspace)?;

    match cli.command {
        Commands::Agent { message, session } => {
            let (agent_loop, context_tools, _cron_service, _bus) =
                build_agent_loop(&config, &workspace)?;

            if let Some(msg) = message {
                // Set context for the CLI session
                let parts: Vec<&str> = session.splitn(2, ':').collect();
                let (channel, chat_id) = if parts.len() == 2 {
                    (parts[0], parts[1])
                } else {
                    ("cli", session.as_str())
                };
                context_tools.set_context(channel, chat_id).await;
                run_single_message(agent_loop, &session, &msg).await?;
            } else {
                run_interactive(agent_loop, context_tools, &session).await?;
            }
        }
        Commands::Serve => {
            run_gateway(&config, &workspace).await?;
        }
        _ => unreachable!(),
    }

    Ok(())
}

/// Create an interrupt flag for a session. Agent loops consume and clear this flag.
fn run_interrupt(session: &str) -> Result<()> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let interrupts_dir = home.join(".nanobot").join("interrupts");
    std::fs::create_dir_all(&interrupts_dir)?;

    let safe = session
        .chars()
        .map(|c| match c {
            '/' | '\\' | ':' | ' ' => '_',
            _ => c,
        })
        .collect::<String>();
    let flag_path = interrupts_dir.join(format!("{safe}.flag"));
    std::fs::write(&flag_path, chrono::Utc::now().to_rfc3339())?;

    println!("Interrupt requested for session '{session}'.");
    println!("Flag file: {}", flag_path.display());
    Ok(())
}

/// Resolve an API key from config, falling back to an environment variable.
fn resolve_api_key(
    provider_cfg: &Option<nanobot_config::ProviderConfig>,
    env_var: &str,
) -> Option<String> {
    provider_cfg
        .as_ref()
        .and_then(|c| c.api_key.clone())
        .filter(|k| !k.is_empty())
        .or_else(|| std::env::var(env_var).ok().filter(|k| !k.is_empty()))
}

/// Resolve builtin skills directory for progressive skill loading.
///
/// Resolution order:
/// 1) `NANOBOT_BUILTIN_SKILLS` env var
/// 2) Repository-relative path (dev builds)
/// 3) Current working directory fallbacks
fn resolve_builtin_skills_dir() -> Option<PathBuf> {
    if let Ok(path) = std::env::var("NANOBOT_BUILTIN_SKILLS") {
        let p = PathBuf::from(path);
        if p.is_dir() {
            return Some(p);
        }
    }

    let mut candidates =
        vec![PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../nanobot/skills")];

    if let Ok(cwd) = std::env::current_dir() {
        candidates.push(cwd.join("nanobot/skills"));
        candidates.push(cwd.join("../nanobot/skills"));
    }

    candidates.into_iter().find(|p| p.is_dir())
}

/// Create a completion model from config, auto-detecting provider by model name.
///
/// Priority:
/// 1. OpenAI with custom apiBase (covers llama.cpp, vLLM, any OpenAI-compatible)
/// 2. Auto-detect by model name prefix (claude-* → Anthropic, gpt-* → OpenAI, etc.)
/// 3. Explicitly configured providers (check for API keys)
/// 4. Ollama (local-first fallback)
#[allow(deprecated)]
fn create_model(config: &nanobot_config::Config) -> Result<CompletionModelHandle<'static>> {
    let model_name = &config.agents.defaults.model;
    let lower = model_name.to_lowercase();

    // 1. If openai provider has a custom apiBase, use it (OpenAI-compatible server)
    if let Some(ref openai_cfg) = config.providers.openai {
        if openai_cfg.api_base.as_ref().is_some_and(|b| !b.is_empty()) {
            let api_key = openai_cfg.api_key.as_deref().unwrap_or("not-needed");
            let mut builder = openai::CompletionsClient::builder().api_key(api_key);
            if let Some(ref base) = openai_cfg.api_base {
                builder = builder.base_url(base);
            }
            let client: openai::CompletionsClient = builder
                .build()
                .map_err(|e| anyhow::anyhow!("Failed to create OpenAI-compatible client: {e}"))?;
            let model = client.completion_model(model_name);
            tracing::info!(
                "Using OpenAI-compatible provider (base: {})",
                openai_cfg.api_base.as_deref().unwrap_or("default")
            );
            return Ok(CompletionModelHandle::new(Arc::new(model)));
        }
    }

    // 1b. Gateway auto-detection by API key prefix
    if let Some(key) = resolve_api_key(&config.providers.openrouter, "OPENROUTER_API_KEY") {
        if key.starts_with("sk-or-") {
            let client: openrouter::Client = openrouter::Client::new(&key)
                .map_err(|e| anyhow::anyhow!("Failed to create OpenRouter client: {e}"))?;
            let model = client.completion_model(model_name);
            tracing::info!("Using OpenRouter provider (detected by API key prefix)");
            return Ok(CompletionModelHandle::new(Arc::new(model)));
        }
    }

    // 2. Auto-detect provider by model name prefix
    // Anthropic: claude-*
    if lower.starts_with("claude-") {
        if let Some(key) = resolve_api_key(&config.providers.anthropic, "ANTHROPIC_API_KEY") {
            let client: anthropic::Client = anthropic::Client::builder()
                .api_key(&key)
                .build()
                .map_err(|e| anyhow::anyhow!("Failed to create Anthropic client: {e}"))?;
            let model = client.completion_model(model_name);
            tracing::info!("Using Anthropic provider");
            return Ok(CompletionModelHandle::new(Arc::new(model)));
        }
    }

    // OpenAI: gpt-*, o1-*, o3-*, o4-*
    if lower.starts_with("gpt-")
        || lower.starts_with("o1-")
        || lower.starts_with("o3-")
        || lower.starts_with("o4-")
    {
        if let Some(key) = resolve_api_key(&config.providers.openai, "OPENAI_API_KEY") {
            let client: openai::CompletionsClient = openai::CompletionsClient::builder()
                .api_key(&key)
                .build()
                .map_err(|e| anyhow::anyhow!("Failed to create OpenAI client: {e}"))?;
            let model = client.completion_model(model_name);
            tracing::info!("Using OpenAI provider");
            return Ok(CompletionModelHandle::new(Arc::new(model)));
        }
    }

    // DeepSeek: deepseek-*
    if lower.starts_with("deepseek-") || lower.starts_with("deepseek_") {
        if let Some(key) = resolve_api_key(&config.providers.deepseek, "DEEPSEEK_API_KEY") {
            let client: deepseek::Client = deepseek::Client::new(&key)
                .map_err(|e| anyhow::anyhow!("Failed to create DeepSeek client: {e}"))?;
            let model = client.completion_model(model_name);
            tracing::info!("Using DeepSeek provider");
            return Ok(CompletionModelHandle::new(Arc::new(model)));
        }
    }

    // Gemini: gemini-*
    if lower.starts_with("gemini-") {
        if let Some(key) = resolve_api_key(&config.providers.gemini, "GEMINI_API_KEY") {
            let client: gemini::Client = gemini::Client::new(key)
                .map_err(|e| anyhow::anyhow!("Failed to create Gemini client: {e}"))?;
            let model = client.completion_model(model_name);
            tracing::info!("Using Gemini provider");
            return Ok(CompletionModelHandle::new(Arc::new(model)));
        }
    }

    // OpenRouter: model names containing "/" (e.g. "meta-llama/llama-3-70b")
    if lower.contains('/') {
        if let Some(key) = resolve_api_key(&config.providers.openrouter, "OPENROUTER_API_KEY") {
            let client: openrouter::Client = openrouter::Client::new(&key)
                .map_err(|e| anyhow::anyhow!("Failed to create OpenRouter client: {e}"))?;
            let model = client.completion_model(model_name);
            tracing::info!("Using OpenRouter provider");
            return Ok(CompletionModelHandle::new(Arc::new(model)));
        }
    }

    // Groq: explicit config (groq models don't have a consistent prefix)
    if let Some(key) = resolve_api_key(&config.providers.groq, "GROQ_API_KEY") {
        if lower.starts_with("groq-")
            || lower.contains("llama")
            || lower.contains("mixtral")
            || config
                .providers
                .groq
                .as_ref()
                .is_some_and(|g| g.api_key.as_ref().is_some_and(|k| !k.is_empty()))
        {
            let client: groq::Client = groq::Client::new(&key)
                .map_err(|e| anyhow::anyhow!("Failed to create Groq client: {e}"))?;
            let model = client.completion_model(model_name);
            tracing::info!("Using Groq provider");
            return Ok(CompletionModelHandle::new(Arc::new(model)));
        }
    }

    // 3. Fallback: if any provider has an API key set, try it via OpenRouter
    if let Some(key) = resolve_api_key(&config.providers.openrouter, "OPENROUTER_API_KEY") {
        let client: openrouter::Client = openrouter::Client::new(&key)
            .map_err(|e| anyhow::anyhow!("Failed to create OpenRouter client: {e}"))?;
        let model = client.completion_model(model_name);
        tracing::info!("Using OpenRouter provider (fallback)");
        return Ok(CompletionModelHandle::new(Arc::new(model)));
    }

    // 4. Ollama (local-first default)
    let mut builder = ollama::Client::builder().api_key(Nothing);
    if let Some(ref ollama_cfg) = config.providers.ollama {
        if let Some(ref base) = ollama_cfg.api_base {
            builder = builder.base_url(base);
        }
    }
    let client: ollama::Client = builder
        .build()
        .map_err(|e| anyhow::anyhow!("Failed to create Ollama client: {e}"))?;
    let model = client.completion_model(model_name);
    tracing::info!("Using Ollama provider (local default)");
    Ok(CompletionModelHandle::new(Arc::new(model)))
}

/// Holds context-aware tools that need set_context() called before each message.
struct ContextTools {
    message_tool: Arc<MessageTool>,
    spawn_tool: Arc<SpawnTool>,
    cron_tool: Arc<CronTool>,
}

impl ContextTools {
    /// Update all context-aware tools with the current channel/chat_id.
    async fn set_context(&self, channel: &str, chat_id: &str) {
        self.message_tool.set_context(channel, chat_id).await;
        self.spawn_tool.set_context(channel, chat_id).await;
        self.cron_tool.set_context(channel, chat_id).await;
    }
}

#[allow(deprecated)]
#[allow(clippy::type_complexity)]
fn build_agent_loop(
    config: &nanobot_config::Config,
    workspace: &Path,
) -> Result<(
    AgentLoop<CompletionModelHandle<'static>>,
    ContextTools,
    Arc<Mutex<CronService>>,
    MessageBus,
)> {
    let defaults = &config.agents.defaults;
    let model = create_model(config)?;

    // Message bus
    let bus = MessageBus::new(128);

    // Sessions directory
    let sessions_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".nanobot")
        .join("sessions");
    let sessions = SessionManager::new(sessions_dir);

    // Context builder (workspace + builtin skills)
    let builtin_skills = resolve_builtin_skills_dir();
    if let Some(ref dir) = builtin_skills {
        tracing::info!("Builtin skills directory: {}", dir.display());
    } else {
        tracing::warn!(
            "Builtin skills directory not found; only workspace skills will be available"
        );
    }
    let context = ContextBuilder::new(workspace, builtin_skills.as_deref());

    // Tool registry
    let mut tools = ToolRegistry::new();
    let allowed_dir: Option<PathBuf> = if config.tools.restrict_to_workspace {
        Some(workspace.to_path_buf())
    } else {
        None
    };
    tools.register(Box::new(ReadFileTool::new(allowed_dir.clone())));
    tools.register(Box::new(WriteFileTool::new(allowed_dir.clone())));
    tools.register(Box::new(EditFileTool::new(allowed_dir.clone())));
    tools.register(Box::new(ListDirTool::new(allowed_dir)));
    tools.register(Box::new(ExecTool::new(
        workspace.to_path_buf(),
        config.tools.exec.timeout_secs,
        config.tools.restrict_to_workspace,
    )));

    // Web tools
    let brave_api_key = if config.tools.web.search.api_key.is_empty() {
        std::env::var("BRAVE_API_KEY").unwrap_or_default()
    } else {
        config.tools.web.search.api_key.clone()
    };
    tools.register(Box::new(WebSearchTool::new(
        brave_api_key,
        config.tools.web.search.max_results,
    )));
    tools.register(Box::new(WebFetchTool::new(50_000)));

    // Message tool
    let message_tool = Arc::new(MessageTool::new(bus.outbound_tx.clone()));
    tools.register(Box::new(ArcToolWrapper(message_tool.clone())));

    // Subagent manager + spawn tool
    let subagent_manager = Arc::new(SubagentManager::new(
        model.clone(),
        workspace.to_path_buf(),
        bus.inbound_tx.clone(),
        config.clone(),
    ));
    let spawn_tool = Arc::new(SpawnTool::new(subagent_manager));
    tools.register(Box::new(ArcToolWrapper(spawn_tool.clone())));

    // Cron service + cron tool
    let cron_store_path = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".nanobot")
        .join("cron")
        .join("jobs.json");
    let cron_service = Arc::new(Mutex::new(CronService::new(
        cron_store_path,
        bus.inbound_tx.clone(),
    )));
    let cron_tool = Arc::new(CronTool::new(cron_service.clone()));
    tools.register(Box::new(ArcToolWrapper(cron_tool.clone())));

    let context_tools = ContextTools {
        message_tool,
        spawn_tool,
        cron_tool,
    };

    let agent_loop = AgentLoop {
        model,
        sessions,
        context,
        tools,
        max_iterations: defaults.max_tool_iterations as usize,
        temperature: defaults.temperature as f64,
        max_tokens: defaults.max_tokens as u64,
        memory_window: defaults.memory_window,
        model_name: defaults.model.clone(),
        model_overrides: ModelOverrides::defaults(),
    };

    Ok((agent_loop, context_tools, cron_service, bus))
}

/// Wrapper to register an `Arc<T: Tool>` in the ToolRegistry (which expects `Box<dyn Tool>`).
struct ArcToolWrapper<T: nanobot_core::tools::Tool>(Arc<T>);

#[async_trait::async_trait]
impl<T: nanobot_core::tools::Tool + 'static> nanobot_core::tools::Tool for ArcToolWrapper<T> {
    fn name(&self) -> &str {
        self.0.name()
    }
    fn description(&self) -> &str {
        self.0.description()
    }
    fn parameters_schema(&self) -> serde_json::Value {
        self.0.parameters_schema()
    }
    async fn execute(&self, params: serde_json::Value) -> anyhow::Result<String> {
        self.0.execute(params).await
    }
}

/// Run the full gateway: channels + agent processing loop + cron + heartbeat.
#[allow(deprecated)]
async fn run_gateway(config: &nanobot_config::Config, workspace: &Path) -> Result<()> {
    tracing::info!("Starting gateway...");

    let (mut agent_loop, context_tools, cron_service, mut bus) =
        build_agent_loop(config, workspace)?;

    // Start cron service
    {
        let mut cron = cron_service.lock().await;
        if let Err(e) = cron.start().await {
            tracing::warn!("Failed to start cron service: {e}");
        }
    }

    // Start heartbeat if enabled
    let mut heartbeat_service: Option<nanobot_core::heartbeat::HeartbeatService> = None;
    if config.heartbeat.enabled {
        let mut heartbeat = nanobot_core::heartbeat::HeartbeatService::new(
            workspace.to_path_buf(),
            bus.inbound_tx.clone(),
            Some(config.heartbeat.interval_secs),
        );
        heartbeat.start();
        tracing::info!("Heartbeat service started");
        heartbeat_service = Some(heartbeat);
    }

    // Set up channel manager
    let outbound_rx = bus.outbound_tx.subscribe();
    let mut channel_manager = ChannelManager::new(outbound_rx);

    // Register Telegram channel if enabled
    if config.channels.telegram.enabled {
        let groq_key = resolve_api_key(&config.providers.groq, "GROQ_API_KEY");
        let transcriber =
            match nanobot_transcribe::create_transcriber(&config.transcription, groq_key) {
                Ok(t) => {
                    tracing::info!("Voice transcription initialized");
                    Some(Arc::from(t))
                }
                Err(e) => {
                    tracing::warn!("Voice transcription unavailable: {e}");
                    None
                }
            };
        match TelegramChannel::new(config.channels.telegram.clone(), transcriber) {
            Ok(tg) => {
                channel_manager.register(Arc::new(tg)).await;
                tracing::info!("Telegram channel registered");
            }
            Err(e) => {
                tracing::error!("Failed to create Telegram channel: {e}");
            }
        }
    }

    // Start all channels (spawns polling + outbound dispatcher)
    let enabled = channel_manager.enabled_channels().await;
    if enabled.is_empty() {
        tracing::warn!("No channels enabled. Configure channels in config.json.");
        tracing::info!("Gateway running with cron/heartbeat only. Press Ctrl-C to stop.");
    } else {
        tracing::info!("Starting channels: {}", enabled.join(", "));
    }
    channel_manager.start_all(bus.inbound_tx.clone()).await?;

    tracing::info!("Gateway running. Press Ctrl-C to stop.");

    // Main inbound processing loop
    loop {
        tokio::select! {
            msg = bus.inbound_rx.recv() => {
                let msg = match msg {
                    Some(m) => m,
                    None => {
                        tracing::info!("Inbound channel closed");
                        break;
                    }
                };

                // System messages from subagents need special routing.
                // chat_id contains "origin_channel:origin_chat_id" to route
                // the response back to the correct destination.
                if msg.channel == "system" {
                    let (origin_channel, origin_chat_id) =
                        if let Some((ch, cid)) = msg.chat_id.split_once(':') {
                            (ch.to_string(), cid.to_string())
                        } else {
                            ("cli".to_string(), msg.chat_id.clone())
                        };

                    let session_key = format!("{origin_channel}:{origin_chat_id}");
                    context_tools
                        .set_context(&origin_channel, &origin_chat_id)
                        .await;

                    // Prefix content with system sender info
                    let system_content =
                        format!("[System: {}] {}", msg.sender_id, msg.content);

                    match agent_loop
                        .process_message(&session_key, &system_content, None)
                        .await
                    {
                        Ok(response) => {
                            if let Err(e) = bus.outbound_tx.send(OutboundMessage {
                                channel: origin_channel,
                                chat_id: origin_chat_id,
                                content: response,
                                reply_to: None,
                                metadata: msg.metadata.clone(),
                            }) {
                                tracing::warn!(
                                    "Failed to publish outbound system response to bus: {e}"
                                );
                            }
                        }
                        Err(e) => {
                            tracing::error!("Error processing system message: {e}");
                            if let Err(send_err) = bus.outbound_tx.send(OutboundMessage {
                                channel: origin_channel,
                                chat_id: origin_chat_id,
                                content: format!(
                                    "Background task completed but I couldn't process the result: {e}"
                                ),
                                reply_to: None,
                                metadata: HashMap::new(),
                            }) {
                                tracing::warn!(
                                    "Failed to publish outbound system-error response to bus: {send_err}"
                                );
                            }
                        }
                    }
                    continue;
                }

                // Update tool context for this message's origin
                context_tools
                    .set_context(&msg.channel, &msg.chat_id)
                    .await;

                let session_key = msg.session_key();

                // Handle slash commands
                let content = msg.content.trim();
                if content == "/new" {
                    // Consolidate memory and start fresh
                    let session = match agent_loop.sessions.get_or_create_checked(&session_key) {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::error!("Failed to load session '{session_key}': {e}");
                            if let Err(send_err) = bus.outbound_tx.send(OutboundMessage {
                                channel: msg.channel.clone(),
                                chat_id: msg.chat_id.clone(),
                                content: format!(
                                    "I couldn't load your session state: {e}. \
        Try checking session file permissions."
                                ),
                                reply_to: None,
                                metadata: HashMap::new(),
                            }) {
                                tracing::warn!(
                                    "Failed to publish session-load error response: {send_err}"
                                );
                            }
                            continue;
                        }
                    };
                    let has_messages = !session.messages.is_empty();
                    if has_messages {
                        agent_loop.consolidate_memory(&session_key, true).await;
                    }
                    let session = match agent_loop.sessions.get_or_create_checked(&session_key) {
                        Ok(s) => s,
                        Err(e) => {
                            tracing::error!(
                                "Failed to reload session '{session_key}' before clear: {e}"
                            );
                            continue;
                        }
                    };
                    session.clear();
                    if let Err(e) = agent_loop.sessions.save(&session_key) {
                        tracing::warn!("Failed to save cleared session '{session_key}': {e}");
                    }
                    agent_loop.sessions.invalidate(&session_key);

                    if let Err(e) = bus.outbound_tx.send(OutboundMessage {
                        channel: msg.channel.clone(),
                        chat_id: msg.chat_id.clone(),
                        content: "New session started. Previous conversation has been saved to memory.".to_string(),
                        reply_to: None,
                        metadata: msg.metadata.clone(),
                    }) {
                        tracing::warn!("Failed to publish /new acknowledgement to bus: {e}");
                    }
                    continue;
                }

                if content == "/help" || content == "/start" {
                    if let Err(e) = bus.outbound_tx.send(OutboundMessage {
                        channel: msg.channel.clone(),
                        chat_id: msg.chat_id.clone(),
                        content: "Hi! I'm nanobot.\n\nSend me a message and I'll respond.\n\nCommands:\n/new - Start a new conversation\n/help - Show this help".to_string(),
                        reply_to: None,
                        metadata: msg.metadata.clone(),
                    }) {
                        tracing::warn!("Failed to publish help response to bus: {e}");
                    }
                    continue;
                }

                // Process through agent
                let media_opt = if msg.media.is_empty() {
                    None
                } else {
                    Some(msg.media.as_slice())
                };
                match agent_loop.process_message(&session_key, content, media_opt).await {
                    Ok(response) => {
                        if let Err(e) = bus.outbound_tx.send(OutboundMessage {
                            channel: msg.channel.clone(),
                            chat_id: msg.chat_id.clone(),
                            content: response,
                            reply_to: None,
                            metadata: msg.metadata.clone(),
                        }) {
                            tracing::warn!("Failed to publish outbound response to bus: {e}");
                        }
                    }
                    Err(e) => {
                        tracing::error!("Error processing message: {e}");
                        if let Err(send_err) = bus.outbound_tx.send(OutboundMessage {
                            channel: msg.channel.clone(),
                            chat_id: msg.chat_id.clone(),
                            content: format!("Sorry, I encountered an error: {e}"),
                            reply_to: None,
                            metadata: HashMap::new(),
                        }) {
                            tracing::warn!(
                                "Failed to publish outbound error response to bus: {send_err}"
                            );
                        }
                    }
                }
            }
            _ = tokio::signal::ctrl_c() => {
                tracing::info!("Shutting down...");
                break;
            }
        }
    }

    // Clean shutdown
    channel_manager.stop_all().await?;
    if let Some(ref mut heartbeat) = heartbeat_service {
        heartbeat.stop();
    }
    {
        let mut cron = cron_service.lock().await;
        cron.stop();
    }
    tracing::info!("Gateway stopped");

    Ok(())
}

#[allow(deprecated)]
async fn run_single_message(
    mut agent_loop: AgentLoop<CompletionModelHandle<'static>>,
    session_key: &str,
    message: &str,
) -> Result<()> {
    let response = agent_loop
        .process_message(session_key, message, None)
        .await?;
    render_markdown(&response);
    Ok(())
}

#[allow(deprecated)]
async fn run_interactive(
    mut agent_loop: AgentLoop<CompletionModelHandle<'static>>,
    context_tools: ContextTools,
    session_key: &str,
) -> Result<()> {
    // Save terminal state for restoration on exit
    #[cfg(unix)]
    let saved_term = save_terminal_state();

    // Set initial context from the session key
    let parts: Vec<&str> = session_key.splitn(2, ':').collect();
    let (channel, chat_id) = if parts.len() == 2 {
        (parts[0], parts[1])
    } else {
        ("cli", session_key)
    };
    context_tools.set_context(channel, chat_id).await;
    let history_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".nanobot")
        .join("history");
    std::fs::create_dir_all(&history_dir)?;
    let history_path = history_dir.join("cli_history");

    let mut rl = DefaultEditor::new()?;
    let _ = rl.load_history(&history_path);

    println!("nanobot interactive mode (type /help for commands, Ctrl-D to quit)");
    println!();

    let result = loop {
        // Flush any pending input before reading
        #[cfg(unix)]
        flush_pending_input();

        let readline = rl.readline("you> ");
        match readline {
            Ok(line) => {
                let input = line.trim();
                if input.is_empty() {
                    continue;
                }

                let _ = rl.add_history_entry(input);

                // Handle exit commands
                if matches!(input, "exit" | "quit" | "/exit" | "/quit" | ":q") {
                    break Ok(());
                }

                // Handle slash commands
                match input {
                    "/help" => {
                        println!("Commands:");
                        println!("  /new   - Start a new conversation (consolidates memory)");
                        println!("  /help  - Show this help");
                        println!(
                            "  interrupt (external): `nanobot interrupt --session {session_key}`"
                        );
                        println!("  /quit  - Exit");
                        println!();
                        continue;
                    }
                    "/new" => {
                        // Consolidate current session before clearing
                        let session = match agent_loop.sessions.get_or_create_checked(session_key) {
                            Ok(s) => s,
                            Err(e) => {
                                tracing::error!("Failed to load session '{session_key}': {e}");
                                println!(
                                    "Could not load session state: {e}\nCheck session file permissions and try again."
                                );
                                println!();
                                continue;
                            }
                        };
                        let has_messages = !session.messages.is_empty();

                        if has_messages {
                            println!("Consolidating memory...");
                            agent_loop.consolidate_memory(session_key, true).await;
                        }

                        let session = match agent_loop.sessions.get_or_create_checked(session_key) {
                            Ok(s) => s,
                            Err(e) => {
                                tracing::error!(
                                    "Failed to reload session '{session_key}' before clear: {e}"
                                );
                                println!("Could not reset session: {e}");
                                println!();
                                continue;
                            }
                        };
                        session.clear();
                        if let Err(e) = agent_loop.sessions.save(session_key) {
                            tracing::warn!("Failed to save cleared session '{session_key}': {e}");
                        }
                        agent_loop.sessions.invalidate(session_key);
                        println!("New session started.");
                        println!();
                        continue;
                    }
                    _ => {}
                }

                // Process message
                match agent_loop.process_message(session_key, input, None).await {
                    Ok(response) => {
                        println!();
                        render_markdown(&response);
                        println!();
                    }
                    Err(e) => {
                        eprintln!("Error: {e}");
                        println!();
                    }
                }
            }
            Err(ReadlineError::Interrupted) => {
                println!("^C");
                continue;
            }
            Err(ReadlineError::Eof) => {
                println!("Goodbye!");
                break Ok(());
            }
            Err(err) => {
                eprintln!("Error: {err}");
                break Ok(());
            }
        }
    };

    let _ = rl.save_history(&history_path);

    // Restore terminal state on exit
    #[cfg(unix)]
    if let Some(ref saved) = saved_term {
        restore_terminal_state(saved);
    }

    result
}

fn prompt_with_default(prompt: &str, default: &str) -> Result<String> {
    use std::io::{self, Write};
    print!("{prompt} [{default}]: ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let trimmed = input.trim();
    if trimmed.is_empty() {
        Ok(default.to_string())
    } else {
        Ok(trimmed.to_string())
    }
}

fn prompt_yes_no(prompt: &str, default_yes: bool) -> Result<bool> {
    use std::io::{self, Write};
    let default = if default_yes { "Y/n" } else { "y/N" };
    print!("{prompt} ({default}): ");
    io::stdout().flush()?;
    let mut input = String::new();
    io::stdin().read_line(&mut input)?;
    let answer = input.trim();
    if answer.is_empty() {
        return Ok(default_yes);
    }
    let lower = answer.to_lowercase();
    Ok(matches!(lower.as_str(), "y" | "yes"))
}

/// Initialize configuration and workspace with templates.
fn run_onboard(config_arg: Option<PathBuf>, non_interactive: bool) -> Result<()> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let nanobot_dir = home.join(".nanobot");
    std::fs::create_dir_all(&nanobot_dir)?;

    // Config file
    let config_path = config_arg.unwrap_or_else(|| nanobot_dir.join("config.json"));
    if config_path.exists() {
        println!("Config already exists: {}", config_path.display());
        println!("To reset, delete it and run `nanobot onboard` again.");
    } else {
        let mut cfg = nanobot_config::Config::default();
        if !non_interactive {
            println!("Interactive setup");
            cfg.agents.defaults.workspace =
                prompt_with_default("Workspace path", &cfg.agents.defaults.workspace)?;
            cfg.agents.defaults.model =
                prompt_with_default("Default model", &cfg.agents.defaults.model)?;

            let enable_tg = prompt_yes_no("Enable Telegram channel?", false)?;
            cfg.channels.telegram.enabled = enable_tg;
            if enable_tg {
                cfg.channels.telegram.token =
                    prompt_with_default("Telegram bot token", &cfg.channels.telegram.token)?;
            }

            let mode = prompt_with_default("Transcription mode (auto/local/groq)", "auto")?;
            cfg.transcription.mode = match mode.to_lowercase().as_str() {
                "local" => nanobot_config::TranscriptionMode::Local,
                "groq" => nanobot_config::TranscriptionMode::Groq,
                _ => nanobot_config::TranscriptionMode::Auto,
            };
        }

        let json = serde_json::to_string_pretty(&cfg)?;
        std::fs::write(&config_path, json)?;
        println!("Created config: {}", config_path.display());
    }

    // Workspace
    let config = load_config(&config_path)?;
    let workspace = resolve_workspace(&config.agents.defaults.workspace);
    std::fs::create_dir_all(&workspace)?;
    println!("Workspace: {}", workspace.display());

    // Create workspace templates
    let templates: &[(&str, &str)] = &[
        (
            "AGENTS.md",
            "# Agent Instructions\n\nCustomize your agent's behavior here.\n",
        ),
        (
            "SOUL.md",
            "# Soul\n\nDefine your agent's personality and communication style.\n",
        ),
        (
            "USER.md",
            "# User Information\n\nTell the agent about yourself: name, location, preferences.\n",
        ),
    ];

    for (filename, content) in templates {
        let path = workspace.join(filename);
        if !path.exists() {
            std::fs::write(&path, content)?;
            println!("  Created {filename}");
        }
    }

    // Memory directory
    let memory_dir = workspace.join("memory");
    std::fs::create_dir_all(&memory_dir)?;

    let memory_file = memory_dir.join("MEMORY.md");
    if !memory_file.exists() {
        std::fs::write(
            &memory_file,
            "# Long-term Memory\n\nThis file stores important information that persists across sessions.\n",
        )?;
        println!("  Created memory/MEMORY.md");
    }

    let history_file = memory_dir.join("HISTORY.md");
    if !history_file.exists() {
        std::fs::write(&history_file, "")?;
        println!("  Created memory/HISTORY.md");
    }

    // Skills directory
    let skills_dir = workspace.join("skills");
    std::fs::create_dir_all(&skills_dir)?;
    println!("  Created skills/");

    println!();
    println!("Setup complete! Next steps:");
    println!(
        "  1. Edit {} to configure your LLM provider",
        config_path.display()
    );
    println!("  2. Run `nanobot agent` to start chatting");
    println!();
    println!("Voice transcription notes:");
    println!(
        "  - Local model files auto-download on first use when transcription.autoDownload=true."
    );
    println!("  - ffmpeg must be installed for local transcription audio conversion.");
    println!("  - Manual model setup (optional): ~/.nanobot/models/parakeet-tdt");
    println!();

    Ok(())
}

/// Show system status and configuration summary.
fn run_status(config_path: &Path) -> Result<()> {
    println!("nanobot status");
    println!();

    // Config
    if config_path.exists() {
        println!("  Config:    {} (found)", config_path.display());
    } else {
        println!(
            "  Config:    {} (not found — run `nanobot onboard`)",
            config_path.display()
        );
        return Ok(());
    }

    let config = load_config(config_path)?;
    let workspace = resolve_workspace(&config.agents.defaults.workspace);

    // Workspace
    if workspace.exists() {
        println!("  Workspace: {} (found)", workspace.display());
    } else {
        println!("  Workspace: {} (not found)", workspace.display());
    }

    // Model
    println!("  Model:     {}", config.agents.defaults.model);
    println!();

    // Providers
    println!("  Providers:");
    print_provider_status("  Ollama", &config.providers.ollama);
    print_provider_status("  OpenAI", &config.providers.openai);
    print_provider_status("  Anthropic", &config.providers.anthropic);
    print_provider_status("  OpenRouter", &config.providers.openrouter);
    print_provider_status("  DeepSeek", &config.providers.deepseek);
    print_provider_status("  Groq", &config.providers.groq);
    print_provider_status("  Gemini", &config.providers.gemini);
    println!();

    // Tools
    let brave_key = if config.tools.web.search.api_key.is_empty() {
        std::env::var("BRAVE_API_KEY").unwrap_or_default()
    } else {
        config.tools.web.search.api_key.clone()
    };
    println!("  Tools:");
    println!(
        "    Brave Search: {}",
        if brave_key.is_empty() {
            "not configured"
        } else {
            "configured"
        }
    );
    println!(
        "    Workspace restriction: {}",
        if config.tools.restrict_to_workspace {
            "on"
        } else {
            "off"
        }
    );
    println!("    Exec timeout: {}s", config.tools.exec.timeout_secs);
    println!();

    // Transcription
    println!("  Transcription:");
    println!("    Mode: {:?}", config.transcription.mode);
    let model_path = config.transcription.model_path.clone().unwrap_or_else(|| {
        let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
        home.join(".nanobot/models/parakeet-tdt")
            .to_string_lossy()
            .to_string()
    });
    println!(
        "    Model: {} ({})",
        model_path,
        if nanobot_transcribe::model_files_exist(&model_path) {
            "found"
        } else {
            "not found"
        }
    );
    println!(
        "    ffmpeg: {}",
        if nanobot_transcribe::audio::ffmpeg_available() {
            "available"
        } else {
            "not found"
        }
    );
    println!(
        "    Execution provider: {}",
        config
            .transcription
            .execution_provider
            .as_deref()
            .unwrap_or("cpu")
    );
    println!(
        "    Auto download: {}",
        if config.transcription.auto_download {
            "enabled"
        } else {
            "disabled"
        }
    );
    if let Some(ref url) = config.transcription.model_url {
        if !url.is_empty() {
            println!("    Model URL: {url}");
        }
    }

    Ok(())
}

fn print_provider_status(label: &str, provider: &Option<nanobot_config::ProviderConfig>) {
    if let Some(p) = provider {
        let has_key = p.api_key.as_ref().is_some_and(|k| !k.is_empty());
        let has_base = p.api_base.as_ref().is_some_and(|b| !b.is_empty());
        if has_key && has_base {
            let base = p.api_base.as_deref().unwrap_or("");
            println!("  {label}: key set, base: {base}");
        } else if has_key {
            println!("  {label}: key set");
        } else if has_base {
            let base = p.api_base.as_deref().unwrap_or("");
            println!("  {label}: base: {base}");
        } else {
            println!("  {label}: configured (empty)");
        }
    }
}

/// Handle cron CLI subcommands.
async fn run_cron_command(action: CronCommands, config: &nanobot_config::Config) -> Result<()> {
    use nanobot_core::cron::{CronSchedule, ScheduleKind};

    let store_path = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".nanobot")
        .join("cron")
        .join("jobs.json");

    // Create a dummy inbound_tx — CLI cron commands don't send messages
    let (inbound_tx, _inbound_rx) = tokio::sync::mpsc::channel(1);
    let mut cron_service = CronService::new(store_path, inbound_tx);
    cron_service.start().await?;

    match action {
        CronCommands::List { all } => {
            let jobs = cron_service.list_jobs(all);
            if jobs.is_empty() {
                println!("No scheduled jobs.");
                return Ok(());
            }
            println!(
                "{:<10} {:<8} {:<20} {:<15} Next Run",
                "ID", "Enabled", "Name", "Schedule"
            );
            println!("{}", "-".repeat(75));
            for job in &jobs {
                let schedule_desc = match job.schedule.kind {
                    ScheduleKind::Every => {
                        let secs = job.schedule.every_ms.unwrap_or(0) / 1000;
                        if secs >= 3600 {
                            format!("every {}h", secs / 3600)
                        } else if secs >= 60 {
                            format!("every {}m", secs / 60)
                        } else {
                            format!("every {}s", secs)
                        }
                    }
                    ScheduleKind::Cron => job.schedule.expr.clone().unwrap_or_else(|| "?".into()),
                    ScheduleKind::At => match job.schedule.at_ms {
                        Some(ms) => chrono::DateTime::from_timestamp_millis(ms)
                            .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                            .unwrap_or_else(|| "?".into()),
                        None => "?".into(),
                    },
                };
                let next_run = match job.state.next_run_at_ms {
                    Some(ms) => chrono::DateTime::from_timestamp_millis(ms)
                        .map(|dt| dt.format("%Y-%m-%d %H:%M").to_string())
                        .unwrap_or_else(|| "—".into()),
                    None => "—".into(),
                };
                println!(
                    "{:<10} {:<8} {:<20} {:<15} {}",
                    job.id,
                    if job.enabled { "yes" } else { "no" },
                    &job.name[..job.name.len().min(20)],
                    schedule_desc,
                    next_run
                );
            }
        }
        CronCommands::Add {
            name,
            message,
            every,
            cron,
            at,
            deliver,
            channel,
            to,
        } => {
            let (schedule, delete_after_run) = if let Some(secs) = every {
                (
                    CronSchedule {
                        kind: ScheduleKind::Every,
                        at_ms: None,
                        every_ms: Some(secs as i64 * 1000),
                        expr: None,
                        tz: None,
                    },
                    false,
                )
            } else if let Some(expr) = cron {
                (
                    CronSchedule {
                        kind: ScheduleKind::Cron,
                        at_ms: None,
                        every_ms: None,
                        expr: Some(expr),
                        tz: None,
                    },
                    false,
                )
            } else if let Some(at_str) = at {
                let dt = chrono::DateTime::parse_from_rfc3339(&at_str).map_err(|e| {
                    anyhow::anyhow!(
                        "Invalid datetime (use RFC3339 format, e.g. 2025-06-01T09:00:00Z): {e}"
                    )
                })?;
                (
                    CronSchedule {
                        kind: ScheduleKind::At,
                        at_ms: Some(dt.timestamp_millis()),
                        every_ms: None,
                        expr: None,
                        tz: None,
                    },
                    true,
                )
            } else {
                anyhow::bail!("Must specify one of --every, --cron, or --at");
            };

            let job = cron_service.add_job(
                &name,
                schedule,
                &message,
                deliver,
                channel,
                to,
                delete_after_run,
            )?;
            println!("Added job '{}' (id: {})", job.name, job.id);
        }
        CronCommands::Remove { job_id } => {
            if cron_service.remove_job(&job_id) {
                println!("Removed job {job_id}");
            } else {
                println!("Job {job_id} not found");
            }
        }
        CronCommands::Enable { job_id, disable } => {
            let enabled = !disable;
            match cron_service.enable_job(&job_id, enabled) {
                Some(job) => {
                    println!(
                        "Job '{}' (id: {}) {}",
                        job.name,
                        job.id,
                        if enabled { "enabled" } else { "disabled" }
                    );
                }
                None => {
                    println!("Job {job_id} not found");
                }
            }
        }
        CronCommands::Run { job_id } => {
            // For manual run, we just print the job info — actual execution requires the gateway
            let jobs = cron_service.list_jobs(true);
            match jobs.iter().find(|j| j.id == job_id) {
                Some(job) => {
                    println!(
                        "Job '{}' (id: {}) message: {}",
                        job.name, job.id, job.payload.message
                    );
                    println!(
                        "Note: Manual execution requires the gateway to be running (`nanobot serve`)."
                    );
                }
                None => {
                    println!("Job {job_id} not found");
                }
            }
        }
    }

    let _ = config; // suppress unused warning
    Ok(())
}

/// Handle channel CLI subcommands.
fn run_channel_command(action: ChannelCommands, config: &nanobot_config::Config) -> Result<()> {
    match action {
        ChannelCommands::Status => {
            println!("Channels:");
            println!();

            // Telegram
            let tg = &config.channels.telegram;
            println!("  Telegram:");
            println!("    Enabled: {}", tg.enabled);
            if tg.enabled {
                let token_display = if tg.token.len() > 10 {
                    format!("{}...{}", &tg.token[..4], &tg.token[tg.token.len() - 4..])
                } else if tg.token.is_empty() {
                    "(not set)".into()
                } else {
                    "***".into()
                };
                println!("    Token:   {token_display}");
                if tg.allow_from.is_empty() {
                    println!("    Access:  open (no allowFrom configured)");
                } else {
                    println!("    Access:  restricted to {} user(s)", tg.allow_from.len());
                }
                if let Some(ref proxy) = tg.proxy {
                    println!("    Proxy:   {proxy}");
                }
            }
        }
    }

    Ok(())
}
