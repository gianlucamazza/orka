//! Command-line interface for interacting with Orka services and local tooling.
#![allow(unreachable_pub)]

mod chat_renderer;
mod client;
mod cmd;
mod completion;
mod markdown;
mod media;
mod onboard;
mod prompt;
mod protocol;
mod shell;
mod table;
mod term_caps;
mod util;
mod ws_discovery;

use clap::{CommandFactory, Parser};
use tracing_subscriber::EnvFilter;

const VERSION_LONG: &str = concat!(
    env!("CARGO_PKG_VERSION"),
    " (",
    env!("ORKA_GIT_SHA"),
    " ",
    env!("ORKA_BUILD_DATE"),
    ")"
);

#[derive(Parser)]
#[command(name = "orka", version = VERSION_LONG, about = "Orka AI platform CLI")]
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
    /// Initialize Orka with a guided LLM-driven configuration wizard
    Init {
        /// LLM provider (anthropic, openai, google, ollama, custom)
        #[arg(long)]
        provider: Option<String>,
        /// API key (skip interactive prompt)
        #[arg(long)]
        api_key: Option<String>,
        /// Model override
        #[arg(long)]
        model: Option<String>,
        /// Base URL (for ollama/custom OpenAI-compatible providers)
        #[arg(long)]
        base_url: Option<String>,
        /// Output path for the generated config
        #[arg(long, default_value = "orka.toml")]
        output: String,
        /// Generate minimal config without LLM conversation
        #[arg(long)]
        minimal: bool,
        /// Extend existing config instead of overwriting
        #[arg(long)]
        extend: bool,
    },
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
    /// Pair a mobile device with this Orka server
    Mobile {
        #[command(subcommand)]
        action: MobileAction,
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
    /// Comprehensive system diagnostics
    Doctor {
        #[command(subcommand)]
        action: Option<cmd::doctor::DoctorAction>,
        /// Output format
        #[arg(long, default_value = "text", value_enum)]
        format: cmd::doctor::OutputFormat,
        /// Filter by category
        #[arg(long, value_enum)]
        category: Option<cmd::doctor::Category>,
        /// Run a specific check by ID (e.g., CFG-001)
        #[arg(long)]
        check: Option<String>,
        /// Minimum severity to report
        #[arg(long, default_value = "info", value_enum)]
        min_severity: cmd::doctor::Severity,
        /// Show verbose details (also enables provider reachability probe)
        #[arg(long, short)]
        verbose: bool,
        /// Attempt auto-remediation with interactive confirmation
        #[arg(long)]
        fix: bool,
        /// Path to orka.toml config file
        #[arg(long)]
        config: Option<String>,
        /// Per-check timeout in seconds
        #[arg(long, default_value = "5")]
        timeout: u64,
    },
    /// Generate shell completions
    Completions {
        /// Shell to generate completions for
        shell: clap_complete::Shell,
    },
    /// Show version information
    Version,
    /// Update orka to the latest release
    Update {
        /// Check for available updates without installing
        #[arg(long)]
        check: bool,
    },
    /// Real-time TUI dashboard
    Dashboard {
        /// Polling interval in seconds
        #[arg(long, default_value = "2")]
        interval: u64,
    },
    /// Manage research campaigns, runs, candidates, and promotions
    Research {
        #[command(subcommand)]
        action: ResearchAction,
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
    /// Send a task via A2A `message/send`
    Send {
        /// Task JSON (or plain text message)
        task: String,
    },
    /// Stream a task via A2A `message/stream` (SSE)
    Stream {
        /// Task text
        task: String,
    },
    /// Task management subcommands
    Tasks {
        #[command(subcommand)]
        action: TasksAction,
    },
}

#[derive(clap::Subcommand)]
enum MobileAction {
    /// Create a QR pairing session and wait for the phone to complete it
    Pair {
        /// Maximum time to wait for pairing completion, in seconds
        #[arg(long, default_value = "120")]
        timeout: u64,
    },
}

#[derive(clap::Subcommand)]
enum ResearchAction {
    /// Manage research campaigns
    Campaign {
        #[command(subcommand)]
        action: ResearchCampaignAction,
    },
    /// Manage research runs
    Run {
        #[command(subcommand)]
        action: ResearchRunAction,
    },
    /// Manage research candidates
    Candidate {
        #[command(subcommand)]
        action: ResearchCandidateAction,
    },
    /// Manage promotion requests
    Promotion {
        #[command(subcommand)]
        action: ResearchPromotionAction,
    },
}

/// Arguments for `research campaign create`.
#[derive(clap::Args)]
struct ResearchCampaignCreate {
    /// Campaign name
    name: String,
    /// Workspace name
    #[arg(long)]
    workspace: String,
    /// Repository path
    #[arg(long)]
    repo_path: String,
    /// Baseline git ref
    #[arg(long)]
    baseline_ref: String,
    /// Task description
    #[arg(long)]
    task: String,
    /// Verification command
    #[arg(long)]
    verify: String,
    /// Additional context
    #[arg(long)]
    context: Option<String>,
    /// Editable file paths (repeatable)
    #[arg(long = "path")]
    editable_paths: Vec<String>,
    /// Metric name to track
    #[arg(long)]
    metric_name: Option<String>,
    /// Regex to extract metric value
    #[arg(long)]
    metric_regex: Option<String>,
    /// Metric direction (higher/lower)
    #[arg(long, default_value = "higher")]
    direction: String,
    /// Baseline metric value
    #[arg(long)]
    baseline_metric: Option<f64>,
    /// Minimum required improvement
    #[arg(long)]
    min_improvement: Option<f64>,
    /// Cron expression for scheduled runs
    #[arg(long)]
    cron: Option<String>,
    /// Target branch for promotions
    #[arg(long, default_value = "main")]
    target_branch: String,
}

#[derive(clap::Subcommand)]
enum ResearchCampaignAction {
    /// List all research campaigns
    List,
    /// Show details for a campaign
    Show {
        /// Campaign ID
        id: String,
    },
    /// Create a new research campaign
    Create(Box<ResearchCampaignCreate>),
    /// Delete a campaign
    Delete {
        /// Campaign ID
        id: String,
        /// Skip confirmation prompt
        #[arg(long, short = 'y')]
        yes: bool,
    },
    /// Pause a campaign
    Pause {
        /// Campaign ID
        id: String,
    },
    /// Resume a paused campaign
    Resume {
        /// Campaign ID
        id: String,
    },
    /// Trigger a new run for a campaign
    Run {
        /// Campaign ID
        id: String,
    },
}

#[derive(clap::Subcommand)]
enum ResearchRunAction {
    /// List runs (optionally filtered by campaign)
    List {
        /// Filter by campaign ID
        #[arg(long)]
        campaign_id: Option<String>,
    },
    /// Show details for a run
    Show {
        /// Run ID
        id: String,
    },
}

#[derive(clap::Subcommand)]
enum ResearchCandidateAction {
    /// List candidates (optionally filtered by campaign)
    List {
        /// Filter by campaign ID
        #[arg(long)]
        campaign_id: Option<String>,
    },
    /// Show details for a candidate
    Show {
        /// Candidate ID
        id: String,
    },
    /// Promote a candidate (creates a promotion request if target branch set)
    Promote {
        /// Candidate ID
        id: String,
        /// Approve automatically
        #[arg(long)]
        approve: bool,
    },
}

#[derive(clap::Subcommand)]
enum ResearchPromotionAction {
    /// List promotion requests (optionally filtered by campaign)
    List {
        /// Filter by campaign ID
        #[arg(long)]
        campaign_id: Option<String>,
    },
    /// Show details for a promotion request
    Show {
        /// Promotion request ID
        id: String,
    },
    /// Approve a promotion request
    Approve {
        /// Promotion request ID
        id: String,
    },
    /// Reject a promotion request
    Reject {
        /// Promotion request ID
        id: String,
        /// Rejection reason
        #[arg(long)]
        reason: Option<String>,
    },
}

#[derive(clap::Subcommand)]
enum TasksAction {
    /// Get a task by ID
    Get { task_id: String },
    /// List tasks (optional state filter)
    List {
        /// Filter by state (e.g. working, completed, failed)
        #[arg(long)]
        state: Option<String>,
    },
    /// Cancel a task by ID
    Cancel { task_id: String },
}

#[tokio::main]
#[allow(clippy::too_many_lines)]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("warn")),
        )
        .init();

    let cli = Cli::parse();
    let api_key = cli.api_key.as_deref();
    let server_client = client::OrkaClient::new(&cli.server, api_key);
    let make_adapter = || client::OrkaClient::new(&cli.adapter, api_key);

    let result = match cli.command {
        Commands::Init {
            provider,
            api_key,
            model,
            base_url,
            output,
            minimal,
            extend,
        } => {
            cmd::init::run(cmd::init::InitArgs {
                provider,
                api_key,
                model,
                base_url,
                output,
                minimal,
                extend,
            })
            .await
        }
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
                ws_discovery::discover()
            };
            cmd::send::run(
                &make_adapter(),
                &text,
                session_id.as_deref(),
                timeout,
                ws,
                !no_workspace,
            )
            .await
        }
        Commands::Chat {
            session_id,
            no_workspace,
        } => {
            let ws = if no_workspace {
                None
            } else {
                ws_discovery::discover()
            };
            cmd::chat::run(
                &make_adapter(),
                &server_client,
                session_id.as_deref(),
                ws,
                !no_workspace,
            )
            .await
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
            WorkspaceAction::List => cmd::workspace::list(&server_client).await,
            WorkspaceAction::Show { name } => cmd::workspace::show(&server_client, &name).await,
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
        Commands::Mobile { action } => match action {
            MobileAction::Pair { timeout } => cmd::mobile::run_pair(&server_client, timeout).await,
        },
        Commands::A2a { action } => match action {
            A2aAction::Card => cmd::a2a::card(&server_client).await,
            A2aAction::Send { task } => cmd::a2a::send(&server_client, &task).await,
            A2aAction::Stream { task } => cmd::a2a::stream(&server_client, &task).await,
            A2aAction::Tasks { action } => match action {
                TasksAction::Get { task_id } => cmd::a2a::tasks_get(&server_client, &task_id).await,
                TasksAction::List { state } => {
                    cmd::a2a::tasks_list(&server_client, state.as_deref()).await
                }
                TasksAction::Cancel { task_id } => {
                    cmd::a2a::tasks_cancel(&server_client, &task_id).await
                }
            },
        },
        Commands::Secret { action } => match action {
            SecretAction::Set { path } => cmd::secret::set(&server_client, &path).await,
            SecretAction::Get { path, reveal } => {
                cmd::secret::get(&server_client, &path, reveal).await
            }
            SecretAction::List => cmd::secret::list(&server_client).await,
            SecretAction::Delete { path } => cmd::secret::delete(&server_client, &path).await,
        },
        Commands::McpServe { config } => cmd::mcp_serve::run(config.as_deref()).await,
        Commands::Config { action } => match action {
            ConfigAction::Check { config } => cmd::config::check(config.as_deref()).await,
            ConfigAction::Migrate { config, dry_run } => {
                cmd::config::migrate_cmd(config.as_deref(), dry_run).await
            }
        },
        Commands::Doctor {
            action,
            format,
            category,
            check,
            min_severity,
            verbose,
            fix,
            config,
            timeout,
        } => match action {
            Some(cmd::doctor::DoctorAction::List) => cmd::doctor::list_checks(),
            Some(cmd::doctor::DoctorAction::Explain { id }) => cmd::doctor::explain_check(&id),
            None => {
                cmd::doctor::run(
                    config.as_deref(),
                    format,
                    category,
                    check.as_deref(),
                    min_severity,
                    verbose,
                    fix,
                    timeout,
                )
                .await
            }
        },
        Commands::Completions { shell } => {
            clap_complete::generate(shell, &mut Cli::command(), "orka", &mut std::io::stdout());
            Ok(())
        }
        Commands::Version => {
            println!("orka {VERSION_LONG}");
            Ok(())
        }
        Commands::Update { check } => {
            if check {
                cmd::update::run_check().await
            } else {
                cmd::update::run_update().await
            }
        }
        Commands::Dashboard { interval } => cmd::dashboard::run(&server_client, interval).await,
        Commands::Research { action } => match action {
            ResearchAction::Campaign { action } => match action {
                ResearchCampaignAction::List => cmd::research::campaign_list(&server_client).await,
                ResearchCampaignAction::Show { id } => {
                    cmd::research::campaign_show(&server_client, &id).await
                }
                ResearchCampaignAction::Create(a) => {
                    cmd::research::campaign_create(
                        &server_client,
                        &a.name,
                        &a.workspace,
                        &a.repo_path,
                        &a.baseline_ref,
                        &a.task,
                        &a.verify,
                        a.context.as_deref(),
                        &a.editable_paths,
                        a.metric_name.as_deref(),
                        a.metric_regex.as_deref(),
                        &a.direction,
                        a.baseline_metric,
                        a.min_improvement,
                        a.cron.as_deref(),
                        &a.target_branch,
                    )
                    .await
                }
                ResearchCampaignAction::Delete { id, yes } => {
                    cmd::research::campaign_delete(&server_client, &id, yes).await
                }
                ResearchCampaignAction::Pause { id } => {
                    cmd::research::campaign_pause(&server_client, &id).await
                }
                ResearchCampaignAction::Resume { id } => {
                    cmd::research::campaign_resume(&server_client, &id).await
                }
                ResearchCampaignAction::Run { id } => {
                    cmd::research::campaign_run(&server_client, &id).await
                }
            },
            ResearchAction::Run { action } => match action {
                ResearchRunAction::List { campaign_id } => {
                    cmd::research::run_list(&server_client, campaign_id.as_deref()).await
                }
                ResearchRunAction::Show { id } => {
                    cmd::research::run_show(&server_client, &id).await
                }
            },
            ResearchAction::Candidate { action } => match action {
                ResearchCandidateAction::List { campaign_id } => {
                    cmd::research::candidate_list(&server_client, campaign_id.as_deref()).await
                }
                ResearchCandidateAction::Show { id } => {
                    cmd::research::candidate_show(&server_client, &id).await
                }
                ResearchCandidateAction::Promote { id, approve } => {
                    cmd::research::candidate_promote(&server_client, &id, approve).await
                }
            },
            ResearchAction::Promotion { action } => match action {
                ResearchPromotionAction::List { campaign_id } => {
                    cmd::research::promotion_list(&server_client, campaign_id.as_deref()).await
                }
                ResearchPromotionAction::Show { id } => {
                    cmd::research::promotion_show(&server_client, &id).await
                }
                ResearchPromotionAction::Approve { id } => {
                    cmd::research::promotion_approve(&server_client, &id).await
                }
                ResearchPromotionAction::Reject { id, reason } => {
                    cmd::research::promotion_reject(&server_client, &id, reason.as_deref()).await
                }
            },
        },
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}
