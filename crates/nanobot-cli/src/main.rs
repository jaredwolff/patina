use std::path::{Path, PathBuf};
use std::sync::Arc;

use anyhow::Result;
use clap::{Parser, Subcommand};
use nanobot_config::{find_config_path, load_config, resolve_workspace};
use nanobot_core::agent::{AgentLoop, ContextBuilder};
use nanobot_core::session::SessionManager;
use nanobot_core::tools::filesystem::{EditFileTool, ListDirTool, ReadFileTool, WriteFileTool};
use nanobot_core::tools::shell::ExecTool;
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
    },
    /// Start gateway with all enabled channels
    Serve,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env().unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

    // Load config
    let config_path = cli.config.unwrap_or_else(find_config_path);
    let config = load_config(&config_path)?;

    // Resolve workspace
    let workspace = resolve_workspace(&config.agents.defaults.workspace);
    std::fs::create_dir_all(&workspace)?;

    match cli.command {
        Commands::Agent { message } => {
            let agent_loop = build_agent_loop(&config, &workspace)?;

            if let Some(msg) = message {
                run_single_message(agent_loop, &msg).await?;
            } else {
                run_interactive(agent_loop).await?;
            }
        }
        Commands::Serve => {
            tracing::info!("Starting gateway...");
            eprintln!("Gateway mode not yet implemented. Use `nanobot agent` for CLI mode.");
        }
    }

    Ok(())
}

/// Create a completion model from config, selecting provider based on what's configured.
///
/// Priority: openai (covers llama.cpp, vLLM, any OpenAI-compatible) → ollama → ollama default
#[allow(deprecated)]
fn create_model(config: &nanobot_config::Config) -> Result<CompletionModelHandle<'static>> {
    let model_name = &config.agents.defaults.model;

    // 1. If openai provider is configured with an apiBase, use it (OpenAI-compatible: llama.cpp, vLLM, etc.)
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
    let context = ContextBuilder::new(workspace);

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
    message: &str,
) -> Result<()> {
    let response = agent_loop.process_message("cli:direct", message).await?;
    println!("{response}");
    Ok(())
}

#[allow(deprecated)]
async fn run_interactive(mut agent_loop: AgentLoop<CompletionModelHandle<'static>>) -> Result<()> {
    // History file
    let history_path = dirs::home_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join(".nanobot")
        .join("cli_history.txt");

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

                // Handle slash commands
                match input {
                    "/help" => {
                        println!("Commands:");
                        println!("  /new   - Start a new conversation");
                        println!("  /help  - Show this help");
                        println!("  /quit  - Exit");
                        println!();
                        continue;
                    }
                    "/quit" | "/exit" => {
                        break;
                    }
                    "/new" => {
                        let session = agent_loop.sessions.get_or_create("cli:interactive");
                        session.clear();
                        let _ = agent_loop.sessions.save("cli:interactive");
                        println!("New session started.");
                        println!();
                        continue;
                    }
                    _ => {}
                }

                // Process message
                match agent_loop.process_message("cli:interactive", input).await {
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
