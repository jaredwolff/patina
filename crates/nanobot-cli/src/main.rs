use std::path::PathBuf;

use anyhow::Result;
use clap::{Parser, Subcommand};
use nanobot_config::{find_config_path, load_config, resolve_workspace};
use nanobot_core::agent::{AgentLoop, ContextBuilder};
use nanobot_core::session::SessionManager;
use nanobot_core::tools::filesystem::{EditFileTool, ListDirTool, ReadFileTool, WriteFileTool};
use nanobot_core::tools::shell::ExecTool;
use nanobot_core::tools::ToolRegistry;
use rig::client::{CompletionClient, Nothing};
use rig::providers::ollama;
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
                // Single message mode
                run_single_message(agent_loop, &msg).await?;
            } else {
                // Interactive REPL
                run_interactive(agent_loop).await?;
            }
        }
        Commands::Serve => {
            tracing::info!("Starting gateway...");
            // TODO: load config, start channels + agent loop
            eprintln!("Gateway mode not yet implemented. Use `nanobot agent` for CLI mode.");
        }
    }

    Ok(())
}

fn build_agent_loop(
    config: &nanobot_config::Config,
    workspace: &PathBuf,
) -> Result<AgentLoop<impl rig::completion::CompletionModel>> {
    let defaults = &config.agents.defaults;

    // Create Ollama client (local-first)
    let ollama_client: ollama::Client = ollama::Client::new(Nothing)
        .map_err(|e| anyhow::anyhow!("Failed to create Ollama client: {e}"))?;
    let model = ollama_client.completion_model(&defaults.model);

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
    let allowed_dir = if config.tools.restrict_to_workspace {
        Some(workspace.clone())
    } else {
        None
    };
    tools.register(Box::new(ReadFileTool::new(allowed_dir.clone())));
    tools.register(Box::new(WriteFileTool::new(allowed_dir.clone())));
    tools.register(Box::new(EditFileTool::new(allowed_dir.clone())));
    tools.register(Box::new(ListDirTool::new(allowed_dir)));
    tools.register(Box::new(ExecTool::new(
        workspace.clone(),
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

async fn run_single_message(
    mut agent_loop: AgentLoop<impl rig::completion::CompletionModel>,
    message: &str,
) -> Result<()> {
    let response = agent_loop.process_message("cli:direct", message).await?;
    println!("{response}");
    Ok(())
}

async fn run_interactive(
    mut agent_loop: AgentLoop<impl rig::completion::CompletionModel>,
) -> Result<()> {
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
                // Ctrl-C: cancel current input, continue
                println!("^C");
                continue;
            }
            Err(ReadlineError::Eof) => {
                // Ctrl-D: exit
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
