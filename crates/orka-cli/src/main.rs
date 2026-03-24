mod client;
mod cmd;
mod completion;
mod markdown;
mod prompt;
mod protocol;
mod shell;
mod table;
mod util;
mod workspace;

use clap::{CommandFactory, Parser};
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
    /// Show server status and dependency health
    Status {
        /// Minimal output with exit code 1 if not healthy (for
        /// scripting/probes)
        #[arg(long)]
        short: bool,
    },
    /// Send a single message
    Send {
        /// Message text
        text: String,
        /// Session ID (auto-generated if omitted)
        #[arg(long)]
        session_id: Option<String>,
        /// Timeout in seconds to wait for a reply
        #[arg(long, default_value = "120")]
        timeout: u64,
        /// Skip local workspace discovery (SOUL.md/TOOLS.md)
        #[arg(long)]
        no_workspace: bool,
    },
    /// Interactive chat session
    Chat {
        /// Session ID (auto-generated if omitted)
        #[arg(long)]
        session_id: Option<String>,
        /// Skip local workspace discovery (SOUL.md/TOOLS.md)
        #[arg(long)]
        no_workspace: bool,
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
    /// List and inspect registered skills
    Skill {
        #[command(subcommand)]
        action: SkillAction,
    },
    /// Manage scheduled tasks
    Schedule {
        #[command(subcommand)]
        action: ScheduleAction,
    },
    /// List and inspect server workspaces
    Workspace {
        #[command(subcommand)]
        action: WorkspaceAction,
    },
    /// Inspect the agent graph topology
    Graph {
        /// Output as Graphviz DOT format
        #[arg(long)]
        dot: bool,
    },
    /// Experience / self-learning system
    Experience {
        #[command(subcommand)]
        action: ExperienceAction,
    },
    /// Manage active sessions
    Session {
        #[command(subcommand)]
        action: SessionAction,
    },
    /// Show Prometheus metrics
    Metrics {
        /// Filter metrics by name fragment
        #[arg(long)]
        filter: Option<String>,
        /// Output as JSON
        #[arg(long)]
        json: bool,
    },
    /// Agent-to-agent (A2A) protocol
    A2a {
        #[command(subcommand)]
        action: A2aAction,
    },
    /// Run as MCP server (stdio transport) for Claude Code/Cursor
    McpServe {
        /// Path to orka.toml config file
        #[arg(long)]
        config: Option<String>,
    },
    /// Config validation and migration
    Config {
        #[command(subcommand)]
        action: ConfigAction,
    },
    /// Check sudo NOPASSWD configuration
    Sudo {
        /// Path to orka.toml config file
        #[arg(long)]
        config: Option<String>,
    },
    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
    },
    /// Show version information
    Version {
        /// Check for available updates
        #[arg(long)]
        check: bool,
    },
    /// Update orka to the latest release
    Update,
    /// Real-time TUI dashboard
    Dashboard {
        /// Polling interval in seconds
        #[arg(long, default_value = "2")]
        interval: u64,
    },
}

#[derive(clap::Subcommand)]
enum ConfigAction {
    /// Validate config and show version + warnings
    Check {
        /// Path to orka.toml
        #[arg(long)]
        config: Option<String>,
    },
    /// Apply pending migrations (writes backup + migrated file)
    Migrate {
        /// Path to orka.toml
        #[arg(long)]
        config: Option<String>,
        /// Show diff without writing
        #[arg(long)]
        dry_run: bool,
    },
}

#[derive(clap::Subcommand)]
enum SecretAction {
    /// Set a secret value (value is read from stdin to avoid shell-history
    /// exposure)
    Set {
        /// Secret path (e.g. llm/anthropic)
        path: String,
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
    Purge {
        /// Skip confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
    },
}

#[derive(clap::Subcommand)]
enum SkillAction {
    /// List all registered skills
    List,
    /// Show details for a skill (schema, description)
    Describe {
        /// Skill name
        name: String,
    },
    /// Run skill evaluation scenarios from .eval.toml files
    Eval {
        /// Only evaluate a specific skill by name
        #[arg(long)]
        skill: Option<String>,
        /// Directory containing .eval.toml files (default: evals/)
        #[arg(long)]
        dir: Option<String>,
        /// Output results as JSON
        #[arg(long)]
        json: bool,
    },
}

#[derive(clap::Subcommand)]
enum ScheduleAction {
    /// List all scheduled tasks
    List,
    /// Create a new scheduled task
    Create {
        /// Schedule name
        name: String,
        /// Cron expression (e.g. "0 0 9 * * *")
        #[arg(long)]
        cron: Option<String>,
        /// One-shot ISO-8601 datetime (e.g. "2026-03-20T09:00:00Z")
        #[arg(long)]
        run_at: Option<String>,
        /// Skill to invoke
        #[arg(long)]
        skill: Option<String>,
        /// Skill args as JSON object (e.g. '{"key":"value"}')
        #[arg(long)]
        args: Option<String>,
        /// Message payload (alternative to skill)
        #[arg(long)]
        message: Option<String>,
    },
    /// Delete a scheduled task
    Delete {
        /// Schedule ID
        id: String,
        /// Skip confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
    },
}

#[derive(clap::Subcommand)]
enum WorkspaceAction {
    /// List all server-side workspaces
    List,
    /// Show details for a workspace
    Show {
        /// Workspace name
        name: String,
    },
}

#[derive(clap::Subcommand)]
enum ExperienceAction {
    /// Show experience system status
    Status,
    /// Retrieve learned principles
    Principles {
        /// Workspace to query
        #[arg(long, default_value = "default")]
        workspace: String,
        /// Semantic search query
        #[arg(long, default_value = "")]
        query: String,
        /// Maximum number of principles to return
        #[arg(long, default_value = "10")]
        limit: usize,
    },
    /// Trigger offline distillation
    Distill {
        /// Workspace to distill
        #[arg(long, default_value = "default")]
        workspace: String,
    },
}

#[derive(clap::Subcommand)]
enum SessionAction {
    /// List active sessions
    List {
        /// Maximum number of sessions to return
        #[arg(long, default_value = "20")]
        limit: usize,
    },
    /// Show details for a session
    Show {
        /// Session ID
        id: String,
    },
    /// Delete a session
    Delete {
        /// Session ID
        id: String,
        /// Skip confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
    },
}

#[derive(clap::Subcommand)]
enum A2aAction {
    /// Show the agent card (capabilities)
    Card,
    /// Send a task via A2A JSON-RPC
    Send {
        /// Task JSON (or plain text message)
        task: String,
    },
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
        Commands::Status { short } => cmd::status::run(&server_client, short).await,
        Commands::Send {
            text,
            session_id,
            timeout,
            no_workspace,
        } => {
            let ws = if no_workspace {
                None
            } else {
                workspace::discover()
            };
            cmd::send::run(&adapter_client, &text, session_id.as_deref(), timeout, ws).await
        }
        Commands::Chat {
            session_id,
            no_workspace,
        } => {
            let ws = if no_workspace {
                None
            } else {
                workspace::discover()
            };
            cmd::chat::run(&adapter_client, session_id.as_deref(), ws).await
        }
        Commands::Dlq { action } => match action {
            DlqAction::List => cmd::dlq::list(&server_client).await,
            DlqAction::Replay { id } => cmd::dlq::replay(&server_client, &id).await,
            DlqAction::Purge { yes } => cmd::dlq::purge(&server_client, yes).await,
        },
        Commands::Skill { action } => match action {
            SkillAction::List => cmd::skill::list(&server_client).await,
            SkillAction::Describe { name } => cmd::skill::describe(&server_client, &name).await,
            SkillAction::Eval { skill, dir, json } => {
                cmd::skill::eval(&server_client, skill.as_deref(), dir.as_deref(), json).await
            }
        },
        Commands::Schedule { action } => match action {
            ScheduleAction::List => cmd::schedule::list(&server_client).await,
            ScheduleAction::Create {
                name,
                cron,
                run_at,
                skill,
                args,
                message,
            } => {
                cmd::schedule::create(
                    &server_client,
                    &name,
                    cron.as_deref(),
                    run_at.as_deref(),
                    skill.as_deref(),
                    args.as_deref(),
                    message.as_deref(),
                )
                .await
            }
            ScheduleAction::Delete { id, yes } => {
                cmd::schedule::delete(&server_client, &id, yes).await
            }
        },
        Commands::Workspace { action } => match action {
            WorkspaceAction::List => cmd::workspace_cmd::list(&server_client).await,
            WorkspaceAction::Show { name } => cmd::workspace_cmd::show(&server_client, &name).await,
        },
        Commands::Graph { dot } => cmd::graph::show(&server_client, dot).await,
        Commands::Experience { action } => match action {
            ExperienceAction::Status => cmd::experience::status(&server_client).await,
            ExperienceAction::Principles {
                workspace,
                query,
                limit,
            } => cmd::experience::principles(&server_client, &workspace, &query, limit).await,
            ExperienceAction::Distill { workspace } => {
                cmd::experience::distill(&server_client, &workspace).await
            }
        },
        Commands::Session { action } => match action {
            SessionAction::List { limit } => cmd::session::list(&server_client, limit).await,
            SessionAction::Show { id } => cmd::session::show(&server_client, &id).await,
            SessionAction::Delete { id, yes } => {
                cmd::session::delete(&server_client, &id, yes).await
            }
        },
        Commands::Metrics { filter, json } => {
            cmd::metrics::show(&server_client, filter.as_deref(), json).await
        }
        Commands::A2a { action } => match action {
            A2aAction::Card => cmd::a2a::card(&server_client).await,
            A2aAction::Send { task } => cmd::a2a::send(&server_client, &task).await,
        },
        Commands::Secret { action } => match action {
            SecretAction::Set { path } => cmd::secret::set(&path).await,
            SecretAction::Get { path, reveal } => cmd::secret::get(&path, reveal).await,
            SecretAction::List => cmd::secret::list().await,
            SecretAction::Delete { path } => cmd::secret::delete(&path).await,
        },
        Commands::McpServe { config } => cmd::mcp_serve::run(config.as_deref()).await,
        Commands::Config { action } => match action {
            ConfigAction::Check { config } => cmd::config::check(config.as_deref()).await,
            ConfigAction::Migrate { config, dry_run } => {
                cmd::config::migrate_cmd(config.as_deref(), dry_run).await
            }
        },
        Commands::Sudo { config } => cmd::sudo::check(config.as_deref()).await,
        Commands::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "orka", &mut std::io::stdout());
            Ok(())
        }
        Commands::Version { check } => {
            if check {
                cmd::update::run_check().await
            } else {
                println!("orka {}", env!("CARGO_PKG_VERSION"));
                Ok(())
            }
        }
        Commands::Update => cmd::update::run_update().await,
        Commands::Dashboard { interval } => cmd::dashboard::run(&server_client, interval).await,
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
