use clap::{Parser, Subcommand};

#[derive(Parser)]
#[command(name = "nanobot", about = "Lightweight AI agent", version)]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Run interactive CLI chat
    Agent {
        /// Single message mode
        #[arg(short, long)]
        message: Option<String>,
    },
    /// Start gateway with all enabled channels
    Serve,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "info".into()),
        )
        .init();

    let cli = Cli::parse();

    match cli.command {
        Commands::Agent { message } => {
            if let Some(msg) = message {
                tracing::info!("Single message mode: {msg}");
                // TODO: process single message via agent loop
            } else {
                tracing::info!("Interactive mode");
                // TODO: rustyline REPL
            }
        }
        Commands::Serve => {
            tracing::info!("Starting gateway...");
            // TODO: load config, start channels + agent loop
        }
    }

    Ok(())
}
