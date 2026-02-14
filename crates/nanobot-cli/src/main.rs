use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use nanobot_config::{find_config_path, load_config, resolve_workspace};
use nanobot_core::agent::{AgentLoop, ContextBuilder};
use nanobot_core::session::SessionManager;
use nanobot_core::tools::filesystem::{EditFileTool, ListDirTool, ReadFileTool, WriteFileTool};
use nanobot_core::tools::shell::ExecTool;
use nanobot_core::tools::web::{WebFetchTool, WebSearchTool};
use nanobot_core::tools::ToolRegistry;
use rig::client::completion::CompletionModelHandle;
use rig::client::{CompletionClient, Nothing};
use rig::providers::{ollama, openai};
use rustyline::error::ReadlineError;
use rustyline::DefaultEditor;

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
    Onboard,
    /// Show system status and configuration
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
        Commands::Onboard => {
            return run_onboard(cli.config);
        }
        Commands::Status => {
            let config_path = cli.config.unwrap_or_else(find_config_path);
            return run_status(&config_path);
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
            let agent_loop = build_agent_loop(&config, &workspace)?;

            if let Some(msg) = message {
                run_single_message(agent_loop, &session, &msg).await?;
            } else {
                run_interactive(agent_loop, &session).await?;
            }
        }
        Commands::Serve => {
            tracing::info!("Starting gateway...");
            eprintln!("Gateway mode not yet implemented. Use `nanobot agent` for CLI mode.");
        }
        _ => unreachable!(),
    }

    Ok(())
}

/// Create a completion model from config, selecting provider based on what's configured.
///
/// Priority: openai (covers llama.cpp, vLLM, any OpenAI-compatible) -> ollama -> ollama default
#[allow(deprecated)]
fn create_model(config: &nanobot_config::Config) -> Result<CompletionModelHandle<'static>> {
    let model_name = &config.agents.defaults.model;

    // 1. If openai provider is configured with an apiBase, use it (OpenAI-compatible)
    if let Some(ref openai_cfg) = config.providers.openai {
        if openai_cfg.api_base.is_some() || openai_cfg.api_key.is_some() {
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

    // 2. Ollama (local-first default)
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
    tracing::info!("Using Ollama provider");
    Ok(CompletionModelHandle::new(Arc::new(model)))
}

#[allow(deprecated)]
fn build_agent_loop(
    config: &nanobot_config::Config,
    workspace: &Path,
) -> Result<AgentLoop<CompletionModelHandle<'static>>> {
    let defaults = &config.agents.defaults;
    let model = create_model(config)?;

    // Sessions directory
    let sessions_dir = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".nanobot")
        .join("sessions");
    let sessions = SessionManager::new(sessions_dir);

    // Context builder
    let context = ContextBuilder::new(workspace, None);

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

    Ok(AgentLoop {
        model,
        sessions,
        context,
        tools,
        max_iterations: defaults.max_tool_iterations as usize,
        temperature: defaults.temperature as f64,
        max_tokens: defaults.max_tokens as u64,
        memory_window: defaults.memory_window,
    })
}

#[allow(deprecated)]
async fn run_single_message(
    mut agent_loop: AgentLoop<CompletionModelHandle<'static>>,
    session_key: &str,
    message: &str,
) -> Result<()> {
    let response = agent_loop.process_message(session_key, message).await?;
    println!("{response}");
    Ok(())
}

#[allow(deprecated)]
async fn run_interactive(
    mut agent_loop: AgentLoop<CompletionModelHandle<'static>>,
    session_key: &str,
) -> Result<()> {
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

    loop {
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
                    break;
                }

                // Handle slash commands
                match input {
                    "/help" => {
                        println!("Commands:");
                        println!("  /new   - Start a new conversation (consolidates memory)");
                        println!("  /help  - Show this help");
                        println!("  /quit  - Exit");
                        println!();
                        continue;
                    }
                    "/new" => {
                        // Consolidate current session before clearing
                        let session = agent_loop.sessions.get_or_create(session_key);
                        let has_messages = !session.messages.is_empty();

                        if has_messages {
                            println!("Consolidating memory...");
                            agent_loop.consolidate_memory(session_key, true).await;
                        }

                        let session = agent_loop.sessions.get_or_create(session_key);
                        session.clear();
                        let _ = agent_loop.sessions.save(session_key);
                        agent_loop.sessions.invalidate(session_key);
                        println!("New session started.");
                        println!();
                        continue;
                    }
                    _ => {}
                }

                // Process message
                match agent_loop.process_message(session_key, input).await {
                    Ok(response) => {
                        println!();
                        println!("{response}");
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
                break;
            }
            Err(err) => {
                eprintln!("Error: {err}");
                break;
            }
        }
    }

    let _ = rl.save_history(&history_path);
    Ok(())
}

/// Initialize configuration and workspace with default templates.
fn run_onboard(config_arg: Option<PathBuf>) -> Result<()> {
    let home = dirs::home_dir().unwrap_or_else(|| PathBuf::from("."));
    let nanobot_dir = home.join(".nanobot");
    std::fs::create_dir_all(&nanobot_dir)?;

    // Config file
    let config_path = config_arg.unwrap_or_else(|| nanobot_dir.join("config.json"));
    if config_path.exists() {
        println!("Config already exists: {}", config_path.display());
        println!("To reset, delete it and run `nanobot onboard` again.");
    } else {
        let default_config = nanobot_config::Config::default();
        let json = serde_json::to_string_pretty(&default_config)?;
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
