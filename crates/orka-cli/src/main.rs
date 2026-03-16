mod client;
mod cmd;
mod protocol;

use clap::Parser;
use tracing_subscriber::EnvFilter;

#[derive(Parser)]
#[command(name = "orka", version, about = "Orka AI platform CLI")]
struct Cli {
    /// Main server URL
    #[arg(
        long,
        default_value = "http://127.0.0.1:8080",
        env = "ORKA_SERVER_URL",
        global = true
    )]
    server: String,

    /// Custom adapter URL
    #[arg(
        long,
        default_value = "http://127.0.0.1:8081",
        env = "ORKA_ADAPTER_URL",
        global = true
    )]
    adapter: String,

    /// API key for authentication
    #[arg(long, env = "ORKA_API_KEY", global = true)]
    api_key: Option<String>,

    #[command(subcommand)]
    command: Commands,
}

#[derive(clap::Subcommand)]
enum Commands {
    /// Check server health
    Health,
    /// Show server status
    Status,
    /// Check readiness (exit code 1 if not ready)
    Ready,
    /// Send a single message
    Send {
        /// Message text
        text: String,
        /// Session ID (auto-generated if omitted)
        #[arg(long)]
        session_id: Option<String>,
        /// Timeout in seconds to wait for a reply
        #[arg(long, default_value = "10")]
        timeout: u64,
    },
    /// Interactive chat session
    Chat {
        /// Session ID (auto-generated if omitted)
        #[arg(long)]
        session_id: Option<String>,
    },
    /// Dead letter queue operations
    Dlq {
        #[command(subcommand)]
        action: DlqAction,
    },
    /// Manage secrets in the secret store
    Secret {
        #[command(subcommand)]
        action: SecretAction,
    },
    /// Run as MCP server (stdio transport) for Claude Code/Cursor
    McpServe {
        /// Path to orka.toml config file
        #[arg(long)]
        config: Option<String>,
    },
}

#[derive(clap::Subcommand)]
enum SecretAction {
    /// Set a secret value
    Set {
        /// Secret path (e.g. llm/anthropic)
        path: String,
        /// Secret value
        value: String,
    },
    /// Get a secret value (masked by default)
    Get {
        /// Secret path
        path: String,
        /// Show the full value
        #[arg(long)]
        reveal: bool,
    },
    /// List all secrets
    List,
    /// Delete a secret
    Delete {
        /// Secret path
        path: String,
    },
}

#[derive(clap::Subcommand)]
enum DlqAction {
    /// List messages in the dead letter queue
    List,
    /// Replay a specific message from the DLQ
    Replay {
        /// Message ID to replay
        id: String,
    },
    /// Purge all messages from the DLQ
    Purge,
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();
    let api_key = cli.api_key.as_deref();
    let server_client = client::OrkaClient::new(&cli.server, api_key);
    let adapter_client = client::OrkaClient::new(&cli.adapter, api_key);

    let result = match cli.command {
        Commands::Health => cmd::health::run(&server_client).await,
        Commands::Status => cmd::status::run(&server_client).await,
        Commands::Ready => cmd::ready::run(&server_client).await,
        Commands::Send {
            text,
            session_id,
            timeout,
        } => cmd::send::run(&adapter_client, &text, session_id.as_deref(), timeout).await,
        Commands::Chat { session_id } => {
            cmd::chat::run(&adapter_client, session_id.as_deref()).await
        }
        Commands::Dlq { action } => match action {
            DlqAction::List => cmd::dlq::list(&server_client).await,
            DlqAction::Replay { id } => cmd::dlq::replay(&server_client, &id).await,
            DlqAction::Purge => cmd::dlq::purge(&server_client).await,
        },
        Commands::Secret { action } => match action {
            SecretAction::Set { path, value } => cmd::secret::set(&path, &value).await,
            SecretAction::Get { path, reveal } => cmd::secret::get(&path, reveal).await,
            SecretAction::List => cmd::secret::list().await,
            SecretAction::Delete { path } => cmd::secret::delete(&path).await,
        },
        Commands::McpServe { config } => cmd::mcp_serve::run(config.as_deref()).await,
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
