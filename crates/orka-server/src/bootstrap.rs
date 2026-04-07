//! Server bootstrap: orchestrates all initialization steps and runs until
//! shutdown.

use std::{future::IntoFuture, sync::Arc};

use anyhow::Context;
use orka_config::{
    CodingConfig, CodingProvider, CodingSelectionPolicy, MemoryBackend, OrkaConfig,
    SearchProviderKind, SecretBackend,
};
use orka_core::{
    ConversationArtifact, ConversationArtifactOrigin, ConversationMessage, ConversationMessageRole,
    ConversationMessageStatus, ConversationStatus, Envelope, MediaPayload, MessageId,
    OutboundMessage,
    stream::AgentStopReason,
    traits::{
        ArtifactStore, ConversationStore, DeadLetterQueue, EventSink, Guardrail, MemoryStore,
        MessageBus, PriorityQueue, SecretManager, SessionLock, SessionStore,
    },
};
use orka_gateway::{Gateway, GatewayConfig, GatewayDeps};
use orka_infra::{
    QueueBundle, create_artifact_store, create_bus, create_conversation_store, create_queue,
    create_session_store,
};
use orka_server::{
    mobile_auth::{MobileAuthConfig, MobileAuthService, RedisMobileAuthService},
    router::{
        AdapterInfo, BUILD_DATE, GIT_SHA, MobileEventHub, RouterParams, ServerFeatures, VERSION,
        build_router,
    },
};
use orka_worker::{CommandRegistry, GraphDispatcher, WorkerPool};
use orka_workspace::{WorkspaceLoader, WorkspaceRegistry};
use tokio::task::JoinHandle;
use tokio_util::sync::CancellationToken;
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

use crate::{
    adapters::{AdapterStartArgs, start_all_adapters},
    experience::create_experience_service,
    providers::build_llm_clients,
    scheduler_adapter::SchedulerSkillRegistryAdapter,
    update::spawn_update_check,
};

// ---------------------------------------------------------------------------
// Private infrastructure types
// ---------------------------------------------------------------------------

/// Infrastructure primitives created during step 3.
struct InfraBundle {
    bus: Arc<dyn MessageBus>,
    sessions: Arc<dyn SessionStore>,
    conversations: Arc<dyn ConversationStore>,
    artifacts: Arc<dyn ArtifactStore>,
    queue: Arc<dyn PriorityQueue>,
    dlq: Arc<dyn DeadLetterQueue>,
    memory: Arc<dyn MemoryStore>,
    memory_lock: Arc<dyn SessionLock>,
    secrets: Arc<dyn SecretManager>,
    event_sink: Arc<dyn EventSink>,
}

/// Skill registry and related services created during step 4.
struct SkillBundle {
    skills: Arc<orka_skills::SkillRegistry>,
    soft_skills: Option<Arc<orka_skills::SoftSkillRegistry>>,
    scheduler_store: Option<Arc<dyn orka_scheduler::ScheduleStore>>,
    fact_store: Option<Arc<orka_knowledge::FactStore>>,
    coding_runtime: Option<orka_agent::executor::CodingRuntimeStatus>,
    mcp_server_count: usize,
}

/// Accumulated server state after all initialization phases complete.
struct Bootstrap {
    config: OrkaConfig,
    infra: InfraBundle,
    /// Prometheus handle kept alive here; not in `InfraBundle` because
    /// `PrometheusHandle` is not Clone and must be moved to `RouterParams`.
    metrics_handle: Option<orka_observe::metrics::PrometheusHandle>,
    skill_bundle: SkillBundle,
    llm_client: Option<Arc<dyn orka_llm::LlmClient>>,
    guardrail: Option<Arc<dyn Guardrail>>,
    workspace_registry: Arc<WorkspaceRegistry>,
    shutdown: CancellationToken,
    adapters: Vec<Arc<dyn orka_core::traits::ChannelAdapter>>,
    experience_service: Option<Arc<orka_experience::ExperienceService>>,
    graph: Arc<orka_agent::AgentGraph>,
    commands: Arc<CommandRegistry>,
    gateway_handle: JoinHandle<()>,
    start_time: std::time::Instant,
    stream_registry: orka_core::StreamRegistry,
    mobile_events: MobileEventHub,
    auth_enabled: bool,
    _env_watcher: Option<crate::env_watcher::EnvWatcher>,
}

/// Inputs for setting up the HTTP management endpoint.
struct HttpServerDeps<'a> {
    config: &'a OrkaConfig,
    infra: &'a InfraBundle,
    /// Prometheus handle moved here from `Bootstrap`; not cloneable.
    metrics_handle: Option<orka_observe::metrics::PrometheusHandle>,
    skill_bundle: &'a SkillBundle,
    experience_service: &'a Option<Arc<orka_experience::ExperienceService>>,
    workspace_registry: &'a Arc<WorkspaceRegistry>,
    graph: &'a Arc<orka_agent::AgentGraph>,
    adapters: &'a [Arc<dyn orka_core::traits::ChannelAdapter>],
    a2a_state: Option<orka_a2a::A2aState>,
    agent_directory: Arc<orka_a2a::AgentDirectory>,
    checkpoint_store: Option<Arc<dyn orka_checkpoint::CheckpointStore>>,
    start_time: std::time::Instant,
    auth_enabled: bool,
    stream_registry: orka_core::StreamRegistry,
    mobile_events: MobileEventHub,
    session_cancel_tokens: orka_worker::SessionCancelTokens,
}

// ---------------------------------------------------------------------------
// Helper: select coding backend
// ---------------------------------------------------------------------------

fn select_coding_backend(
    config: &CodingConfig,
    claude_available: bool,
    codex_available: bool,
    opencode_available: bool,
) -> Option<CodingProvider> {
    match config.default_provider {
        CodingProvider::ClaudeCode => claude_available.then_some(CodingProvider::ClaudeCode),
        CodingProvider::Codex => codex_available.then_some(CodingProvider::Codex),
        CodingProvider::OpenCode => opencode_available.then_some(CodingProvider::OpenCode),
        CodingProvider::Auto => match config.selection_policy {
            // claude → codex → opencode (backward-compatible)
            CodingSelectionPolicy::Availability | CodingSelectionPolicy::PreferClaude => {
                claude_available
                    .then_some(CodingProvider::ClaudeCode)
                    .or_else(|| codex_available.then_some(CodingProvider::Codex))
                    .or_else(|| opencode_available.then_some(CodingProvider::OpenCode))
            }
            // codex → claude → opencode
            CodingSelectionPolicy::PreferCodex => codex_available
                .then_some(CodingProvider::Codex)
                .or_else(|| claude_available.then_some(CodingProvider::ClaudeCode))
                .or_else(|| opencode_available.then_some(CodingProvider::OpenCode)),
            // opencode → claude → codex
            CodingSelectionPolicy::PreferOpenCode => opencode_available
                .then_some(CodingProvider::OpenCode)
                .or_else(|| claude_available.then_some(CodingProvider::ClaudeCode))
                .or_else(|| codex_available.then_some(CodingProvider::Codex)),
        },
    }
}

fn to_runtime_memory_config(config: &orka_config::MemoryConfig) -> orka_memory::MemoryConfig {
    let backend = match config.backend {
        MemoryBackend::Auto => orka_memory::config::MemoryBackend::Auto,
        MemoryBackend::Redis => orka_memory::config::MemoryBackend::Redis,
        MemoryBackend::Memory => orka_memory::config::MemoryBackend::Memory,
    };
    let mut runtime = orka_memory::MemoryConfig::default();
    runtime.backend = backend;
    runtime.max_entries = config.max_entries;
    runtime
}

fn to_runtime_secret_config(config: &orka_config::SecretConfig) -> orka_secrets::SecretConfig {
    let backend = match config.backend {
        SecretBackend::Redis => orka_secrets::SecretBackend::Redis,
        SecretBackend::File => orka_secrets::SecretBackend::File,
        _ => orka_secrets::SecretBackend::default(),
    };
    let mut runtime = orka_secrets::SecretConfig::default();
    runtime.backend = backend;
    runtime.file_path.clone_from(&config.file_path);
    runtime
        .encryption_key_path
        .clone_from(&config.encryption_key_path);
    runtime
        .encryption_key_env
        .clone_from(&config.encryption_key_env);
    runtime.redis.url.clone_from(&config.redis.url);
    runtime
}

fn to_runtime_observe_config(config: &orka_config::ObserveConfig) -> orka_observe::ObserveConfig {
    let mut runtime = orka_observe::ObserveConfig::default();
    runtime.enabled = config.enabled;
    runtime.backend.clone_from(&config.backend);
    runtime.otlp_endpoint.clone_from(&config.otlp_endpoint);
    runtime.batch_size = config.batch_size;
    runtime.flush_interval_ms = config.flush_interval_ms;
    runtime.service_name.clone_from(&config.service_name);
    runtime.service_version.clone_from(&config.service_version);
    runtime
}

fn to_runtime_audit_config(config: &orka_config::AuditConfig) -> orka_observe::AuditConfig {
    let mut runtime = orka_observe::AuditConfig::default();
    runtime.enabled = config.enabled;
    runtime.output.clone_from(&config.output);
    runtime.path.clone_from(&config.path);
    runtime.redis_key.clone_from(&config.redis_key);
    runtime
}

fn to_runtime_sandbox_config(config: &orka_config::SandboxConfig) -> orka_wasm::SandboxConfig {
    let mut runtime = orka_wasm::SandboxConfig::default();
    runtime.backend.clone_from(&config.backend);
    runtime.limits.timeout_secs = config.limits.timeout_secs;
    runtime.limits.max_memory_bytes = config.limits.max_memory_bytes;
    runtime.limits.max_output_bytes = config.limits.max_output_bytes;
    runtime.limits.max_open_files = config.limits.max_open_files;
    runtime.limits.max_pids = config.limits.max_pids;
    runtime.allowed_paths.clone_from(&config.allowed_paths);
    runtime.denied_paths.clone_from(&config.denied_paths);
    runtime
}

fn to_runtime_web_config(config: &orka_config::WebConfig) -> orka_web::WebConfig {
    let search_provider = match config.search_provider {
        SearchProviderKind::Tavily => orka_web::SearchProviderKind::Tavily,
        SearchProviderKind::Brave => orka_web::SearchProviderKind::Brave,
        SearchProviderKind::Searxng => orka_web::SearchProviderKind::Searxng,
        SearchProviderKind::None => orka_web::SearchProviderKind::None,
    };
    orka_web::WebConfig {
        search_provider,
        api_key: config.api_key.clone(),
        api_key_env: config.api_key_env.clone(),
        searxng_base_url: config.searxng_base_url.clone(),
        max_results: config.max_results,
        max_read_chars: config.max_read_chars,
        cache_ttl_secs: config.cache_ttl_secs,
        max_content_chars: config.max_content_chars,
        read_timeout_secs: config.read_timeout_secs,
        user_agent: config.user_agent.clone(),
    }
}

fn to_runtime_http_config(config: &orka_config::HttpClientConfig) -> orka_web::HttpClientConfig {
    let mut runtime = orka_web::HttpClientConfig::default();
    runtime.timeout_secs = config.timeout_secs;
    runtime.max_redirects = config.max_redirects;
    runtime.user_agent.clone_from(&config.user_agent);
    runtime.default_headers.clone_from(&config.default_headers);
    runtime.webhooks = config
        .webhooks
        .iter()
        .map(|webhook| {
            let mut runtime_webhook = orka_web::WebhookConfig::default();
            runtime_webhook.name.clone_from(&webhook.name);
            runtime_webhook.url.clone_from(&webhook.url);
            runtime_webhook.method.clone_from(&webhook.method);
            runtime_webhook.secret.clone_from(&webhook.secret);
            runtime_webhook.retry.max_retries = webhook.retry.max_retries;
            runtime_webhook.retry.delay_secs = webhook.retry.delay_secs;
            runtime_webhook
        })
        .collect();
    runtime
}

// ---------------------------------------------------------------------------
// Step 0–2: config loading + tracing
// ---------------------------------------------------------------------------

/// Load + validate configuration, initialise tracing, and spawn the background
/// update checker. Called once at startup before any other step.
fn init_config_and_tracing() -> anyhow::Result<OrkaConfig> {
    // 0. Load .env file (no-op if missing — production uses systemd
    //    EnvironmentFile)
    let _ = dotenvy::dotenv();

    // 1. Load + validate config
    let mut config = OrkaConfig::load(None).context("failed to load configuration")?;
    config
        .validate()
        .context("configuration validation failed")?;

    // 1b. Ensure workspace state dir is in os.allowed_paths for
    // ProtectSystem=strict
    if config.os.enabled {
        let workspace_parent = std::path::Path::new(&config.workspace_dir)
            .parent()
            .map_or_else(
                || config.workspace_dir.clone(),
                |p| p.to_string_lossy().into_owned(),
            );
        if !config
            .os
            .allowed_paths
            .iter()
            .any(|p| workspace_parent.starts_with(p.as_str()))
        {
            config.os.allowed_paths.push(workspace_parent);
        }
    }

    // 2. Init tracing (RUST_LOG takes precedence over config)
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new(config.logging.level.as_str()));
    if config.logging.json {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
    info!(
        version = VERSION,
        git_sha = GIT_SHA,
        build_date = BUILD_DATE,
        "Orka server starting"
    );
    spawn_update_check();
    debug!(?config, "loaded configuration");

    Ok(config)
}

// ---------------------------------------------------------------------------
// Step 3: infrastructure primitives
// ---------------------------------------------------------------------------

/// Create all stateful infrastructure handles: bus, sessions, queues, memory,
/// secrets, event sink, and metrics recorder.
///
/// Returns `(InfraBundle, metrics_handle)`. The Prometheus handle is returned
/// separately because `PrometheusHandle` is not `Clone` and must be moved
/// directly into `RouterParams`.
fn init_infra(
    config: &OrkaConfig,
) -> anyhow::Result<(InfraBundle, Option<orka_observe::metrics::PrometheusHandle>)> {
    let bus = create_bus(&config.bus, &config.redis.url).context("failed to create message bus")?;
    let sessions = create_session_store(&config.session, &config.redis.url)
        .context("failed to create session store")?;
    let conversations = create_conversation_store(&config.redis.url)
        .context("failed to create conversation store")?;
    let artifacts =
        create_artifact_store(&config.redis.url).context("failed to create artifact store")?;
    let QueueBundle { queue, dlq } =
        create_queue(&config.redis.url).context("failed to create priority queue")?;
    let memory_config = to_runtime_memory_config(&config.memory);
    let orka_memory::MemoryBundle {
        store: memory,
        lock: memory_lock,
    } = orka_memory::create_memory_store(&memory_config, &config.redis.url)
        .context("failed to create memory store")?;
    info!("memory store ready");
    let secret_config = to_runtime_secret_config(&config.secrets);
    let secrets = orka_secrets::create_secret_manager(&secret_config, &config.redis.url)
        .context("failed to create secret manager")?;
    info!("secret manager ready");
    let observe_config = to_runtime_observe_config(&config.observe);
    let audit_config = to_runtime_audit_config(&config.audit);
    let event_sink =
        orka_observe::create_event_sink(&observe_config, &audit_config, &config.redis.url);
    info!("event sink ready");

    // 3b. Install Prometheus metrics recorder
    let metrics_handle = orka_observe::metrics::install_prometheus_recorder();
    if metrics_handle.is_some() {
        info!("prometheus metrics recorder installed");
    }

    Ok((
        InfraBundle {
            bus,
            sessions,
            conversations,
            artifacts,
            queue,
            dlq,
            memory,
            memory_lock,
            secrets,
            event_sink,
        },
        metrics_handle,
    ))
}

// ---------------------------------------------------------------------------
// Step 4 sub-registrators
// ---------------------------------------------------------------------------

/// Connect MCP servers in parallel and register each tool as a skill.
/// Returns the number of successfully connected servers.
async fn register_mcp_skills(
    config: &OrkaConfig,
    skills: &mut orka_skills::SkillRegistry,
) -> usize {
    let mut mcp_set = tokio::task::JoinSet::new();
    for server_config in &config.mcp.servers {
        let transport = match (&server_config.command, &server_config.url) {
            (Some(cmd), _) => {
                let mut builder = orka_mcp::McpTransportConfig::stdio(cmd.clone())
                    .args(server_config.args.clone())
                    .envs(server_config.env.clone());
                if let Some(dir) = &server_config.working_dir {
                    builder = builder.working_dir(std::path::PathBuf::from(dir));
                }
                builder.build()
            }
            (None, Some(url)) => {
                let mut builder = orka_mcp::McpTransportConfig::http(url.clone());
                if let Some(auth) = &server_config.auth {
                    builder = builder.auth(orka_mcp::McpOAuthConfig {
                        token_url: auth.token_url.clone(),
                        client_id: auth.client_id.clone(),
                        client_secret_env: auth.client_secret_env.clone(),
                        scopes: auth.scopes.clone(),
                    });
                }
                builder.build()
            }
            (None, None) => {
                tracing::error!(
                    name = %server_config.name,
                    "MCP server entry has neither 'command' nor 'url' — skipping"
                );
                continue;
            }
        };
        let mcp_config = orka_mcp::McpServerConfig {
            name: server_config.name.clone(),
            transport,
        };
        let server_name = server_config.name.clone();
        mcp_set.spawn(async move {
            let client = orka_mcp::McpClient::connect(mcp_config).await?;
            let client = Arc::new(client);
            let tools = client.list_tools().await?;
            Ok::<_, orka_core::Error>((server_name, client, tools))
        });
    }
    let mut count: usize = 0;
    while let Some(result) = mcp_set.join_next().await {
        match result {
            Ok(Ok((server_name, client, tools))) => {
                count += 1;
                for tool in tools {
                    let bridge = orka_mcp::McpToolBridge::new(
                        client.clone(),
                        tool.name.clone(),
                        tool.description.unwrap_or_default(),
                        tool.input_schema,
                    );
                    skills.register(Arc::new(bridge));
                    info!(tool = %tool.name, server = %server_name, "registered MCP tool");
                }
            }
            Ok(Err(e)) => warn!(%e, "failed to connect/list MCP server tools"),
            Err(e) => warn!(%e, "MCP connection task panicked"),
        }
    }
    count
}

/// Register knowledge/RAG and scheduler skills.
/// Returns `(fact_store, scheduler_store)`.
fn register_knowledge_and_scheduler_skills(
    config: &OrkaConfig,
    skills: &mut orka_skills::SkillRegistry,
) -> (
    Option<Arc<orka_knowledge::FactStore>>,
    Option<Arc<dyn orka_scheduler::ScheduleStore>>,
) {
    // 4g. Knowledge/RAG skills
    let fact_store = if config.knowledge.enabled {
        match orka_knowledge::create_knowledge_skills(&config.knowledge) {
            Ok(knowledge_skills) => {
                for skill in knowledge_skills {
                    skills.register(skill);
                }
                match orka_knowledge::create_fact_store(&config.knowledge) {
                    Ok(store) => Some(store),
                    Err(e) => {
                        warn!(%e, "failed to initialize semantic fact store");
                        None
                    }
                }
            }
            Err(e) => {
                warn!(%e, "failed to initialize knowledge skills");
                None
            }
        }
    } else {
        None
    };

    // 4h. Scheduler skills
    let scheduler_store = if config.scheduler.enabled {
        match orka_scheduler::create_scheduler_skills(&config.scheduler, &config.redis.url) {
            Ok((scheduler_skills, store)) => {
                for skill in scheduler_skills {
                    skills.register(skill);
                }
                Some(store)
            }
            Err(e) => {
                warn!(%e, "failed to initialize scheduler skills");
                None
            }
        }
    } else {
        None
    };

    (fact_store, scheduler_store)
}

/// Probe environment capabilities and register OS/coding skills.
/// Returns the coding runtime status if OS skills are enabled.
async fn register_os_skills(
    config: &OrkaConfig,
    skills: &mut orka_skills::SkillRegistry,
) -> Option<orka_agent::executor::CodingRuntimeStatus> {
    if !config.os.enabled {
        return None;
    }
    let caps = orka_os::EnvironmentCapabilities::probe(&config.os).await;
    let claude_code_available =
        config.os.coding.providers.claude_code.enabled && caps.claude_code.available;
    let codex_available = config.os.coding.providers.codex.enabled && caps.codex.available;
    let opencode_available = config.os.coding.providers.opencode.enabled && caps.opencode.available;
    let selected_backend = select_coding_backend(
        &config.os.coding,
        claude_code_available,
        codex_available,
        opencode_available,
    );
    let (file_modifications_allowed, command_execution_allowed) = match selected_backend {
        Some(CodingProvider::ClaudeCode) => (
            config
                .os
                .coding
                .providers
                .claude_code
                .allow_file_modifications,
            config
                .os
                .coding
                .providers
                .claude_code
                .allow_command_execution,
        ),
        Some(CodingProvider::Codex) => (
            config.os.coding.providers.codex.allow_file_modifications,
            config.os.coding.providers.codex.allow_command_execution,
        ),
        Some(CodingProvider::OpenCode) => (
            config.os.coding.providers.opencode.allow_file_modifications,
            config.os.coding.providers.opencode.allow_command_execution,
        ),
        _ => (false, false),
    };
    info!(
        no_new_privileges = caps.no_new_privileges,
        package_updates = caps.package_updates.available,
        systemctl = caps.systemctl.available,
        journalctl = caps.journalctl.available,
        claude_code = caps.claude_code.available,
        codex = caps.codex.available,
        "environment capabilities probed"
    );
    match orka_os::create_os_skills(&config.os, Some(&caps)) {
        Ok(os_skills) => {
            for skill in os_skills {
                skills.register(skill);
            }
        }
        Err(e) => warn!(%e, "failed to initialize OS skills"),
    }
    Some(orka_agent::executor::CodingRuntimeStatus {
        tool_available: config.os.coding.enabled
            && (claude_code_available || codex_available || opencode_available),
        default_provider: config.os.coding.default_provider.to_string(),
        selection_policy: config.os.coding.selection_policy.to_string(),
        claude_code_available,
        codex_available,
        selected_backend: selected_backend.map(|p| p.to_string()),
        file_modifications_allowed,
        command_execution_allowed,
        allowed_paths: config.os.allowed_paths.clone(),
        denied_paths: config.os.denied_paths.clone(),
    })
}

/// Scan and load SKILL.md-based soft skills from the configured directory.
fn register_soft_skills(config: &OrkaConfig) -> Option<Arc<orka_skills::SoftSkillRegistry>> {
    let dir = config.soft_skills.dir.as_ref()?;
    let skills_list = orka_skills::scan_soft_skills(std::path::Path::new(dir));
    let selection_mode =
        orka_skills::SoftSkillSelectionMode::from(config.soft_skills.selection_mode.as_str());
    let mut reg = orka_skills::SoftSkillRegistry::new().with_selection_mode(selection_mode);
    let count = skills_list.len();
    for skill in skills_list {
        reg.register(skill);
    }
    info!(count, selection_mode = %config.soft_skills.selection_mode, "soft skill registry ready");
    Some(Arc::new(reg))
}

// ---------------------------------------------------------------------------
// Steps 4–4l: skill registry
// ---------------------------------------------------------------------------

/// Register all built-in, plugin, and external skills into a [`SkillRegistry`].
/// Returns the finalized registry along with optional sub-services.
async fn init_skills(config: &OrkaConfig) -> anyhow::Result<SkillBundle> {
    // 4. Skill registry
    let mut skills = orka_skills::create_skill_registry();
    skills.register(Arc::new(orka_skills::EchoSkill));

    // 4b. Sandbox + SandboxSkill
    let sandbox_config = to_runtime_sandbox_config(&config.sandbox);
    let sandbox = orka_wasm::create_sandbox(&sandbox_config).context("failed to create sandbox")?;
    skills.register(Arc::new(orka_wasm::SandboxSkill::new(sandbox)));

    // 4c. Shared WASM engine + WASM plugins
    #[cfg(feature = "wasm")]
    {
        let wasm_engine =
            orka_wasm::WasmEngine::new().context("failed to create shared WASM engine")?;
        if let Some(ref plugin_dir) = config.plugins.dir {
            match orka_skills::load_plugins(
                std::path::Path::new(plugin_dir),
                &wasm_engine,
                &config.plugins,
            ) {
                Ok(plugins) => {
                    for plugin in plugins {
                        skills.register(plugin);
                    }
                }
                Err(e) => {
                    warn!(%e, "failed to load plugins");
                }
            }
        }
    }

    // 4d. MCP servers
    let mcp_server_count = register_mcp_skills(config, &mut skills).await;

    // 4e. Web skills
    if config.web.search_provider != SearchProviderKind::None {
        let web_config = to_runtime_web_config(&config.web);
        match orka_web::create_web_skills(&web_config) {
            Ok(web_skills) => {
                for skill in web_skills {
                    skills.register(skill);
                }
            }
            Err(e) => warn!(%e, "failed to initialize web skills"),
        }
    }

    // 4f. HTTP skills
    let http_config = to_runtime_http_config(&config.http);
    match orka_web::create_http_skills(&http_config) {
        Ok(http_skills) => {
            for skill in http_skills {
                skills.register(skill);
            }
        }
        Err(e) => warn!(%e, "failed to initialize HTTP skills"),
    }

    // 4g+4h. Knowledge/RAG and scheduler skills
    let (fact_store, scheduler_store) =
        register_knowledge_and_scheduler_skills(config, &mut skills);

    // 4i. OS skills
    let coding_runtime = register_os_skills(config, &mut skills).await;

    // 4l. Chart skills
    if config.chart.enabled {
        let chart_skills = orka_chart::create_chart_skills();
        let count = chart_skills.len();
        for skill in chart_skills {
            skills.register(skill);
        }
        info!(skill_count = count, "chart skills initialized");
    }

    // 4k. Git skills
    if config.git.enabled {
        match orka_git::create_git_skills(&config.git, None) {
            Ok(git_skills) => {
                let count = git_skills.len();
                for skill in git_skills {
                    skills.register(skill);
                }
                info!(skill_count = count, "git skills initialized");
            }
            Err(e) => warn!(%e, "failed to initialize git skills"),
        }
    }

    let skills = Arc::new(skills);
    info!("skill registry ready ({} skills)", skills.list().len());

    // 4j. Soft skills (SKILL.md-based instruction skills)
    let soft_skills = register_soft_skills(config);

    Ok(SkillBundle {
        skills,
        soft_skills,
        scheduler_store,
        fact_store,
        coding_runtime,
        mcp_server_count,
    })
}

// ---------------------------------------------------------------------------
// Step 6: workspace loading
// ---------------------------------------------------------------------------

/// Load all configured workspaces into a [`WorkspaceRegistry`] and start file
/// watchers for hot-reload.
async fn load_workspaces(config: &OrkaConfig) -> anyhow::Result<Arc<WorkspaceRegistry>> {
    let registry = if config.workspaces.is_empty() {
        let loader = Arc::new(WorkspaceLoader::new(&config.workspace_dir));
        loader.load_all().await?;
        let mut reg = WorkspaceRegistry::new("default".into());
        reg.register("default".into(), loader);
        info!("workspace loaded (single, default)");
        reg
    } else {
        let default_name = config
            .default_workspace
            .clone()
            .unwrap_or_else(|| config.workspaces[0].name.clone());
        let mut reg = WorkspaceRegistry::new(default_name);
        let mut load_set = tokio::task::JoinSet::new();
        let entries = config.workspaces.clone();
        for entry in &entries {
            let loader = Arc::new(WorkspaceLoader::new(&entry.dir));
            let name = entry.name.clone();
            let dir = entry.dir.clone();
            load_set.spawn({
                let loader = loader.clone();
                async move {
                    loader.load_all().await?;
                    Ok::<_, anyhow::Error>((name, dir, loader))
                }
            });
        }
        while let Some(result) = load_set.join_next().await {
            let (name, dir, loader) = result.context("workspace load task panicked")??;
            info!(workspace = %name, dir = %dir, "workspace loaded");
            reg.register(name, loader);
        }
        reg
    };

    let registry = Arc::new(registry);

    // Start workspace file watchers for hot-reload
    for ws_name in registry.list_names() {
        if let Some(loader) = registry.get(ws_name) {
            match orka_workspace::WorkspaceWatcher::start(loader.clone()) {
                Ok(_w) => {}
                Err(e) => warn!(workspace = %ws_name, %e, "failed to start workspace watcher"),
            }
        }
    }
    info!("workspace watchers started");

    Ok(registry)
}

// ---------------------------------------------------------------------------
// Auth layer
// ---------------------------------------------------------------------------

/// Build the auth layer and return whether authentication is enabled.
fn build_auth_layer(config: &OrkaConfig) -> (bool, Option<orka_auth::AuthLayer>) {
    let enabled = config.auth.jwt.is_some() || !config.auth.api_keys.is_empty();
    if enabled {
        use orka_auth::{
            ApiKeyAuthenticator, AuthLayer, Authenticator, CompositeAuthenticator,
            JwtAuthenticator, middleware::AuthMiddlewareConfig,
        };

        let mut backends: Vec<Arc<dyn Authenticator>> = Vec::new();

        if let Some(jwt) = &config.auth.jwt {
            let issuer = jwt.issuer.clone().unwrap_or_else(|| "orka".to_string());
            let audience = jwt.audience.clone();

            if let Some(secret) = &jwt.secret {
                backends.push(Arc::new(JwtAuthenticator::with_secret(
                    issuer, audience, secret,
                )));
            } else if let Some(public_key_path) = &jwt.public_key_path {
                match std::fs::read(public_key_path) {
                    Ok(pem) => match JwtAuthenticator::with_rsa_pem(issuer, audience, &pem) {
                        Ok(authenticator) => backends.push(Arc::new(authenticator)),
                        Err(error) => warn!(
                            %error,
                            path = %public_key_path,
                            "failed to initialize JWT authenticator"
                        ),
                    },
                    Err(error) => warn!(
                        %error,
                        path = %public_key_path,
                        "failed to read JWT public key"
                    ),
                }
            }
        }

        if !config.auth.api_keys.is_empty() {
            backends.push(Arc::new(ApiKeyAuthenticator::new(&config.auth.api_keys)));
        }

        let authenticator = CompositeAuthenticator::new(backends);
        let layer = AuthLayer::new(
            Arc::new(authenticator),
            Arc::new(AuthMiddlewareConfig::default()),
        );
        (true, Some(layer))
    } else {
        (false, None)
    }
}

fn build_mobile_auth_service(config: &OrkaConfig) -> Option<Arc<dyn MobileAuthService>> {
    let jwt = config.auth.jwt.as_ref()?;
    let secret = jwt.secret.as_ref()?;
    let issuer = jwt.issuer.clone().unwrap_or_else(|| "orka".to_string());
    let mobile_auth_config = MobileAuthConfig::new(issuer, jwt.audience.clone(), secret.clone());

    match RedisMobileAuthService::new(&config.redis.url, mobile_auth_config) {
        Ok(service) => Some(Arc::new(
            service.with_public_url(config.server.public_url.clone()),
        )),
        Err(error) => {
            warn!(%error, "mobile auth service unavailable");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Gateway
// ---------------------------------------------------------------------------

/// Spawn the gateway loop and return its join handle.
fn spawn_gateway(
    infra: &InfraBundle,
    workspace_registry: &Arc<WorkspaceRegistry>,
    config: &OrkaConfig,
    shutdown: CancellationToken,
) -> anyhow::Result<JoinHandle<()>> {
    let gateway = Gateway::new(
        GatewayDeps {
            bus: infra.bus.clone(),
            sessions: infra.sessions.clone(),
            queue: infra.queue.clone(),
            workspace: workspace_registry
                .default_loader()
                .context("default workspace not registered")?
                .clone(),
            event_sink: infra.event_sink.clone(),
        },
        GatewayConfig {
            redis_url: Some(config.redis.url.clone()),
            rate_limit: config.gateway.rate_limit,
            dedup_ttl_secs: config.gateway.dedup_ttl_secs,
            ..Default::default()
        },
    );
    Ok(tokio::spawn(async move {
        if let Err(e) = gateway.run(shutdown).await {
            error!(%e, "gateway error");
        }
    }))
}

// ---------------------------------------------------------------------------
// A2A state
// ---------------------------------------------------------------------------

/// Build the Agent-to-Agent state when A2A discovery or known agents are
/// configured.
fn build_a2a_state(
    config: &OrkaConfig,
    skills: &Arc<orka_skills::SkillRegistry>,
    secrets: &Arc<dyn SecretManager>,
) -> Option<orka_a2a::A2aState> {
    let a2a_enabled = config.a2a.discovery_enabled || !config.a2a.known_agents.is_empty();
    if !a2a_enabled {
        return None;
    }

    let base_url = format!("http://{}:{}", config.server.host, config.server.port);
    let agent_card = orka_a2a::build_agent_card_with_auth(
        "orka",
        "Orka AI Agent Platform",
        &base_url,
        skills,
        config.a2a.auth_enabled.then_some(&config.auth),
    );

    let use_redis = config.a2a.store_backend == "redis";
    let (task_store, push_store): (
        Arc<dyn orka_a2a::TaskStore>,
        Arc<dyn orka_a2a::PushNotificationStore>,
    ) = if use_redis {
        let redis_url = &config.redis.url;
        let ts = orka_a2a::RedisTaskStore::new(redis_url).map_or_else(
            |e| {
                tracing::warn!(%e, "failed to create RedisTaskStore, falling back to memory");
                Arc::new(orka_a2a::InMemoryTaskStore::default()) as Arc<dyn orka_a2a::TaskStore>
            },
            |s| Arc::new(s) as Arc<dyn orka_a2a::TaskStore>,
        );
        let ps = orka_a2a::RedisPushNotificationStore::new(redis_url).map_or_else(
            |e| {
                tracing::warn!(%e, "failed to create RedisPushNotificationStore, falling back to memory");
                Arc::new(orka_a2a::InMemoryPushNotificationStore::default())
                    as Arc<dyn orka_a2a::PushNotificationStore>
            },
            |s| Arc::new(s) as Arc<dyn orka_a2a::PushNotificationStore>,
        );
        (ts, ps)
    } else {
        (
            Arc::new(orka_a2a::InMemoryTaskStore::default()),
            Arc::new(orka_a2a::InMemoryPushNotificationStore::default()),
        )
    };

    Some(orka_a2a::A2aState {
        agent_card,
        skills: skills.clone(),
        secrets: secrets.clone(),
        task_store,
        task_events: Arc::default(),
        push_store: push_store.clone(),
        webhook_deliverer: Arc::new(orka_a2a::WebhookDeliverer::new(push_store)),
    })
}

// ---------------------------------------------------------------------------
// A2A discovery
// ---------------------------------------------------------------------------

fn spawn_a2a_discovery(
    config: &OrkaConfig,
    agent_directory: Arc<orka_a2a::AgentDirectory>,
    shutdown: CancellationToken,
) {
    if !config.a2a.known_agents.is_empty() {
        let client = orka_a2a::DiscoveryClient::new(
            config.a2a.known_agents.clone(),
            config.a2a.discovery_interval_secs,
            agent_directory,
        );
        tokio::spawn(client.run(shutdown));
    }
}

// ---------------------------------------------------------------------------
// Worker pool
// ---------------------------------------------------------------------------

fn spawn_worker_pool(
    config: &OrkaConfig,
    infra: &InfraBundle,
    dispatcher: Arc<GraphDispatcher>,
    memory_lock: Arc<dyn SessionLock>,
    shutdown: CancellationToken,
    session_cancel_tokens: orka_worker::SessionCancelTokens,
) -> JoinHandle<()> {
    let worker_pool = WorkerPool::new(
        infra.queue.clone(),
        infra.sessions.clone(),
        infra.bus.clone(),
        dispatcher,
        infra.event_sink.clone(),
        config.worker.concurrency,
        config.queue.max_retries,
    )
    .with_retry_delay(config.worker.retry_base_delay_ms)
    .with_session_lock(memory_lock)
    .with_dlq(infra.dlq.clone())
    .with_session_cancel_tokens(session_cancel_tokens);
    tokio::spawn(async move {
        if let Err(e) = worker_pool.run(shutdown).await {
            error!(%e, "worker pool error");
        }
    })
}

// ---------------------------------------------------------------------------
// Checkpoint store
// ---------------------------------------------------------------------------

fn build_checkpoint_store(redis_url: &str) -> Option<Arc<dyn orka_checkpoint::CheckpointStore>> {
    match orka_checkpoint::RedisCheckpointStore::new_default_ttl(redis_url) {
        Ok(store) => {
            info!("checkpoint store: Redis at {redis_url}");
            Some(Arc::new(store))
        }
        Err(e) => {
            warn!(%e, "checkpoint store unavailable, running without persistence");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// HTTP management endpoint
// ---------------------------------------------------------------------------

/// Collect `AdapterInfo` from the active adapter list, excluding the internal
/// custom adapter which is not user-visible.
fn collect_adapter_infos(
    adapters: &[Arc<dyn orka_core::traits::ChannelAdapter>],
) -> Vec<AdapterInfo> {
    adapters
        .iter()
        .filter(|a| a.channel_id() != "custom")
        .map(|a| AdapterInfo {
            channel_id: a.channel_id().to_string(),
            integration_class: a.integration_class(),
            trust_level: a.trust_level(),
            capabilities: a.capabilities(),
        })
        .collect()
}

/// Build the router parameters from the HTTP server dependencies.
/// Consumes `deps` so that owned fields (`metrics_handle`, `a2a_state`, etc.)
/// are moved into `RouterParams` without cloning.
fn build_router_params(deps: HttpServerDeps<'_>) -> RouterParams {
    let config = deps.config;

    let qdrant_url = if config.knowledge.enabled {
        config.knowledge.vector_store.url.clone()
    } else {
        None
    };

    let entry = deps.graph.entry_agent();
    let agent_name = entry.display_name.clone();
    let agent_model = entry
        .llm_config
        .model
        .clone()
        .unwrap_or_else(|| config.llm.default_model.clone());

    let thinking = entry.llm_config.thinking.as_ref().map(|t| {
        use orka_llm::{ThinkingConfig, ThinkingEffort};
        match t {
            ThinkingConfig::Adaptive { effort } => match effort {
                ThinkingEffort::Low => "low".to_string(),
                ThinkingEffort::Medium => "medium".to_string(),
                ThinkingEffort::High => "high".to_string(),
                ThinkingEffort::Max => "max".to_string(),
            },
            ThinkingConfig::Enabled { budget_tokens } => format!("budget:{budget_tokens}"),
            ThinkingConfig::ReasoningEffort(_) => "reasoning".to_string(),
            _ => "enabled".to_string(),
        }
    });

    let adapter_names = collect_adapter_infos(deps.adapters);

    let coding_backend = deps
        .skill_bundle
        .coding_runtime
        .as_ref()
        .and_then(|r| r.selected_backend.clone());

    let web_search = match config.web.search_provider {
        SearchProviderKind::None => None,
        SearchProviderKind::Tavily => Some("tavily".to_string()),
        SearchProviderKind::Brave => Some("brave".to_string()),
        SearchProviderKind::Searxng => Some("searxng".to_string()),
    };

    let auth_layer = build_auth_layer(config).1;
    let mobile_auth = build_mobile_auth_service(config);

    RouterParams {
        bus: deps.infra.bus.clone(),
        queue: deps.infra.queue.clone(),
        dlq: deps.infra.dlq.clone(),
        skills: deps.skill_bundle.skills.clone(),
        soft_skills: deps.skill_bundle.soft_skills.clone(),
        sessions: deps.infra.sessions.clone(),
        conversations: deps.infra.conversations.clone(),
        artifacts: deps.infra.artifacts.clone(),
        scheduler_store: deps.skill_bundle.scheduler_store.clone(),
        checkpoint_store: deps.checkpoint_store,
        workspace_registry: Arc::clone(deps.workspace_registry),
        graph: Arc::clone(deps.graph),
        experience_service: (*deps.experience_service).clone(),
        start_time: deps.start_time,
        concurrency: config.worker.concurrency,
        redis_url: config.redis.url.clone(),
        qdrant_url,
        auth_layer,
        a2a_state: deps.a2a_state,
        a2a_auth_enabled: config.a2a.auth_enabled,
        agent_directory: deps.agent_directory,
        metrics_handle: deps.metrics_handle,
        agent_name,
        agent_model,
        mcp_server_count: deps.skill_bundle.mcp_server_count,
        features: ServerFeatures {
            knowledge: config.knowledge.enabled,
            scheduler: config.scheduler.enabled,
            experience: config.experience.enabled,
            guardrails: config.guardrails.enabled,
            a2a: config.a2a.discovery_enabled || !config.a2a.known_agents.is_empty(),
            observe: config.observe.enabled,
            research: config.research.enabled,
        },
        thinking,
        agent_count: deps.graph.nodes_iter().count(),
        auth_enabled: deps.auth_enabled,
        adapters: adapter_names,
        coding_backend,
        web_search,
        secret_manager: deps.infra.secrets.clone(),
        research_service: None,
        stream_registry: deps.stream_registry,
        mobile_events: deps.mobile_events,
        controller: Arc::new(orka_core::conversation_controller::ConversationController::new(
            deps.infra.conversations.clone(), deps.infra.bus.clone(), deps.session_cancel_tokens.clone(),
        )),
        mobile_auth,
        mobile_enabled: config.auth.jwt.is_some(),
        mobile_read_rate_limit_per_minute: None,
        mobile_write_rate_limit_per_minute: None,
    }
}

/// Build the router and bind the HTTP management/health endpoint.
async fn start_http_server(deps: HttpServerDeps<'_>) -> anyhow::Result<()> {
    let config = deps.config;
    let router_params = build_router_params(deps);
    let health_app = build_router(router_params);

    let listener =
        tokio::net::TcpListener::bind(format!("{}:{}", config.server.host, config.server.port))
            .await
            .context("failed to bind health endpoint")?;
    info!(
        "health endpoint listening on {}:{}",
        config.server.host, config.server.port
    );
    tokio::spawn(axum::serve(listener, health_app).into_future());
    Ok(())
}

// ---------------------------------------------------------------------------
// Background task spawners
// ---------------------------------------------------------------------------

/// Spawn the scheduler poll loop. Returns `None` if scheduling is not
/// configured.
fn spawn_scheduler_loop(
    config: &OrkaConfig,
    scheduler_store: Option<Arc<dyn orka_scheduler::ScheduleStore>>,
    skills: &Arc<orka_skills::SkillRegistry>,
    event_sink: &Arc<dyn orka_core::traits::EventSink>,
    shutdown: CancellationToken,
) -> Option<JoinHandle<()>> {
    let store = scheduler_store?;
    let scheduler = orka_scheduler::Scheduler::new(
        store,
        Arc::new(SchedulerSkillRegistryAdapter(skills.clone())),
        config.scheduler.poll_interval_secs,
        config.scheduler.max_concurrent,
    )
    .with_event_sink(event_sink.clone());
    Some(tokio::spawn(async move {
        scheduler.run(shutdown).await;
    }))
}

/// Spawn the experience distillation loop. Returns `None` if distillation is
/// disabled or no experience service is configured.
fn spawn_distillation_loop(
    experience_service: Option<Arc<orka_experience::ExperienceService>>,
    workspace_registry: &Arc<WorkspaceRegistry>,
    event_sink: &Arc<dyn EventSink>,
    interval_secs: u64,
    shutdown: CancellationToken,
) -> Option<JoinHandle<()>> {
    let exp = experience_service?;
    if interval_secs == 0 {
        return None;
    }
    let workspace_names: Vec<String> = workspace_registry
        .list_names()
        .into_iter()
        .map(std::string::ToString::to_string)
        .collect();
    let sink = event_sink.clone();
    Some(tokio::spawn(async move {
        let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
        interval.tick().await;
        loop {
            tokio::select! {
                () = shutdown.cancelled() => break,
                _ = interval.tick() => {
                    for ws in &workspace_names {
                        match exp.distill(ws).await {
                            Ok(count) if count > 0 => {
                                info!(workspace = %ws, principles_created = count, "distillation completed");
                                sink.emit(orka_core::DomainEvent::new(
                                    orka_core::DomainEventKind::DistillationCompleted {
                                        workspace: ws.clone(),
                                        principles_created: count,
                                    },
                                )).await;
                            }
                            Ok(_) => {}
                            Err(e) => warn!(workspace = %ws, %e, "distillation failed"),
                        }
                    }
                }
            }
        }
    }))
}

/// Spawn the outbound bridge that routes bus "outbound" messages to the correct
/// adapter by channel ID.
async fn spawn_outbound_bridge(
    bus: &Arc<dyn MessageBus>,
    conversations: Arc<dyn ConversationStore>,
    artifacts: Arc<dyn ArtifactStore>,
    mobile_events: MobileEventHub,
    adapters: Vec<Arc<dyn orka_core::traits::ChannelAdapter>>,
    shutdown: CancellationToken,
) -> anyhow::Result<JoinHandle<()>> {
    let mut outbound_rx = bus.subscribe("outbound").await?;
    Ok(tokio::spawn(async move {
        loop {
            tokio::select! {
                () = shutdown.cancelled() => break,
                msg = outbound_rx.recv() => {
                    if let Some(envelope) = msg {
                        if envelope.channel == "mobile" {
                            if let Err(error) = persist_mobile_outbound(
                                conversations.as_ref(),
                                artifacts.as_ref(),
                                &mobile_events,
                                &envelope,
                            ).await {
                                let conversation_id =
                                    orka_core::ConversationId::from(envelope.session_id);
                                error!(
                                    %error,
                                    conversation_id = %conversation_id,
                                    "failed to persist mobile outbound state"
                                );
                            }
                            continue;
                        }

                        let mut outbound = OutboundMessage::new(
                            envelope.channel.clone(),
                            envelope.session_id,
                            envelope.payload.clone(),
                            None,
                        );
                        outbound.metadata = envelope.metadata.clone();
                        let target = adapters
                            .iter()
                            .find(|a| a.channel_id() == envelope.channel.as_str());
                        if let Some(adapter) = target {
                            if let Err(e) = adapter.send(outbound).await {
                                error!(%e, channel = %envelope.channel, "failed to send outbound message via adapter");
                            }
                        } else {
                            warn!(channel = %envelope.channel, "no adapter found for outbound channel");
                        }
                    } else {
                        warn!("outbound bus channel closed");
                        break;
                    }
                }
            }
        }
    }))
}

/// Result of applying one outbound payload to the transcript.
struct OutboundPayloadResult {
    completed_message: Option<ConversationMessage>,
    last_message_preview: Option<String>,
    updated_at: chrono::DateTime<chrono::Utc>,
}

/// Persist one outbound payload and return the fields that must be written
/// back to `conversation`.  SSE artifact events are emitted directly.
#[allow(clippy::too_many_lines)]
async fn apply_outbound_payload(
    envelope: &Envelope,
    conversation: &orka_core::Conversation,
    assistant_message_id: MessageId,
    stop_reason: Option<AgentStopReason>,
    conversations: &dyn ConversationStore,
    artifacts: &dyn ArtifactStore,
    mobile_events: &MobileEventHub,
) -> anyhow::Result<OutboundPayloadResult> {
    let payload = &envelope.payload;
    let conversation_id = orka_core::ConversationId::from(envelope.session_id);
    let now = chrono::Utc::now();
    match payload {
        orka_core::Payload::Text(text) if !text.trim().is_empty() => {
            let existing = conversations
                .get_message(&conversation_id, &assistant_message_id)
                .await?;
            let mut message = mobile_assistant_message(
                envelope,
                conversation_id,
                assistant_message_id,
                text,
                stop_reason,
            );
            if let Some(existing) = existing {
                message.artifacts = existing.artifacts;
                message.created_at = existing.created_at;
            }
            conversations.upsert_message(&message).await?;
            let preview = Some(preview_text(&message.text));
            let updated_at = message.created_at;
            Ok(OutboundPayloadResult {
                completed_message: Some(message),
                last_message_preview: preview,
                updated_at,
            })
        }
        orka_core::Payload::Media(media) => {
            let mut artifact = persist_mobile_assistant_artifact(
                artifacts,
                conversation,
                assistant_message_id,
                media,
            )
            .await?;
            let mut message = conversations
                .get_message(&conversation_id, &assistant_message_id)
                .await?
                .unwrap_or_else(|| {
                    mobile_assistant_message(
                        envelope,
                        conversation_id,
                        assistant_message_id,
                        "",
                        stop_reason,
                    )
                });
            artifact.message_id = Some(message.id);
            artifacts.update_artifact(&artifact).await?;
            message.artifacts.push(artifact.clone());
            conversations.upsert_message(&message).await?;
            let preview = message
                .text
                .trim()
                .is_empty()
                .then(|| format!("[{}] {}", artifact.mime_type, artifact.filename));
            mobile_events
                .publish(
                    conversation_id,
                    orka_contracts::RealtimeEvent::ArtifactReady {
                        conversation_id: conversation_id.as_uuid(),
                        artifact: serde_json::to_value(&artifact).unwrap_or_default(),
                    },
                )
                .await;
            Ok(OutboundPayloadResult {
                completed_message: None,
                last_message_preview: preview,
                updated_at: now,
            })
        }
        orka_core::Payload::RichInput(input) => {
            let text = input.text.as_deref().unwrap_or("").trim();
            let existing = conversations
                .get_message(&conversation_id, &assistant_message_id)
                .await?;
            let mut message = mobile_assistant_message(
                envelope,
                conversation_id,
                assistant_message_id,
                text,
                stop_reason,
            );
            if let Some(existing) = existing {
                message.artifacts = existing.artifacts;
                message.created_at = existing.created_at;
            }
            for media in &input.attachments {
                let mut artifact = persist_mobile_assistant_artifact(
                    artifacts,
                    conversation,
                    assistant_message_id,
                    media,
                )
                .await?;
                artifact.message_id = Some(message.id);
                artifacts.update_artifact(&artifact).await?;
                message.artifacts.push(artifact.clone());
                mobile_events
                    .publish(
                        conversation_id,
                        orka_contracts::RealtimeEvent::ArtifactReady {
                            conversation_id: conversation_id.as_uuid(),
                            artifact: serde_json::to_value(&artifact).unwrap_or_default(),
                        },
                    )
                    .await;
            }
            conversations.upsert_message(&message).await?;
            let preview = if text.is_empty() {
                message
                    .artifacts
                    .first()
                    .map(|a| format!("[{}] {}", a.mime_type, a.filename))
            } else {
                Some(preview_text(text))
            };
            let updated_at = message.created_at;
            Ok(OutboundPayloadResult {
                completed_message: Some(message),
                last_message_preview: preview,
                updated_at,
            })
        }
        _ => Ok(OutboundPayloadResult {
            completed_message: None,
            last_message_preview: None,
            updated_at: now,
        }),
    }
}

async fn persist_mobile_outbound(
    conversations: &dyn ConversationStore,
    artifacts: &dyn ArtifactStore,
    mobile_events: &MobileEventHub,
    envelope: &Envelope,
) -> anyhow::Result<()> {
    let conversation_id = orka_core::ConversationId::from(envelope.session_id);
    let stop_reason = mobile_stop_reason(envelope);

    let Some(mut conversation) = conversations.get_conversation(&conversation_id).await? else {
        warn!(conversation_id = %conversation_id, "mobile outbound without conversation metadata");
        return Ok(());
    };

    let assistant_message_id = mobile_assistant_message_id(envelope);
    let result = apply_outbound_payload(
        envelope,
        &conversation,
        assistant_message_id,
        stop_reason,
        conversations,
        artifacts,
        mobile_events,
    )
    .await?;

    conversation.updated_at = result.updated_at;
    if let Some(preview) = result.last_message_preview {
        conversation.last_message_preview = Some(preview);
    }
    conversation.status = match stop_reason {
        Some(AgentStopReason::Error) => ConversationStatus::Failed,
        Some(AgentStopReason::Interrupted) => ConversationStatus::Interrupted,
        _ => ConversationStatus::Active,
    };
    conversations.put_conversation(&conversation).await?;

    match stop_reason {
        Some(AgentStopReason::Error) => {
            let error_text = result
                .completed_message
                .as_ref()
                .map(|item| item.text.clone())
                .filter(|value| !value.trim().is_empty())
                .unwrap_or_else(|| "agent execution terminated with error".to_string());
            mobile_events
                .publish(
                    conversation_id,
                    orka_contracts::RealtimeEvent::MessageFailed {
                        conversation_id: conversation_id.as_uuid(),
                        error: error_text,
                    },
                )
                .await;
        }
        _ => {
            if let Some(message) = result.completed_message {
                mobile_events
                    .publish(
                        conversation_id,
                        orka_contracts::RealtimeEvent::MessageCompleted {
                            conversation_id: message.conversation_id.as_uuid(),
                            message: serde_json::to_value(&message).unwrap_or_default(),
                        },
                    )
                    .await;
            }
        }
    }

    Ok(())
}

fn mobile_assistant_message(
    envelope: &Envelope,
    conversation_id: orka_core::ConversationId,
    message_id: MessageId,
    text: &str,
    stop_reason: Option<AgentStopReason>,
) -> ConversationMessage {
    let mut message = ConversationMessage::new(
        message_id,
        conversation_id,
        envelope.session_id,
        ConversationMessageRole::Assistant,
        text.to_string(),
    );
    if matches!(stop_reason, Some(AgentStopReason::Error)) {
        message.status = ConversationMessageStatus::Failed;
        message.finalized_at = None;
    }
    message
}

async fn persist_mobile_assistant_artifact(
    artifacts: &dyn ArtifactStore,
    conversation: &orka_core::Conversation,
    message_id: MessageId,
    media: &MediaPayload,
) -> anyhow::Result<ConversationArtifact> {
    let bytes = media
        .decode_data()
        .ok_or_else(|| anyhow::anyhow!("mobile assistant artifacts require inline data"))?;
    let mut artifact = ConversationArtifact::new(
        conversation.user_id.clone(),
        ConversationArtifactOrigin::AssistantOutput,
        media.mime_type.clone(),
        media
            .filename
            .clone()
            .unwrap_or_else(|| default_filename_for_mime(&media.mime_type)),
    );
    artifact.conversation_id = Some(conversation.id);
    artifact.message_id = Some(message_id);
    artifact.caption = media.caption.clone();
    artifact.size_bytes = media.size_bytes.or(Some(bytes.len() as u64));
    artifacts.put_artifact(&artifact, &bytes).await?;
    Ok(artifact)
}

fn mobile_assistant_message_id(envelope: &Envelope) -> MessageId {
    envelope
        .metadata
        .get("assistant_message_id")
        .cloned()
        .and_then(|value| serde_json::from_value::<String>(value).ok())
        .and_then(|value| uuid::Uuid::parse_str(&value).ok())
        .map_or(envelope.id, MessageId::from)
}

fn mobile_stop_reason(envelope: &Envelope) -> Option<AgentStopReason> {
    envelope
        .metadata
        .get("stop_reason")
        .cloned()
        .and_then(|value| serde_json::from_value(value).ok())
}

fn preview_text(text: &str) -> String {
    const MAX_CHARS: usize = 60;
    let trimmed = text.trim();
    let truncated = trimmed.chars().take(MAX_CHARS).collect::<String>();
    if trimmed.chars().count() > MAX_CHARS {
        format!("{truncated}...")
    } else {
        truncated
    }
}

fn default_filename_for_mime(mime: &str) -> String {
    if let Some(exts) = mime_guess::get_mime_extensions_str(mime)
        && let Some(ext) = exts.first()
    {
        return format!("artifact.{ext}");
    }
    "artifact.bin".to_string()
}

// ---------------------------------------------------------------------------
// Shutdown sequence
// ---------------------------------------------------------------------------

/// Signal readiness, wait for SIGINT/SIGTERM, drain the queue, and join all
/// background tasks.
async fn wait_for_shutdown(
    config: &OrkaConfig,
    shutdown: CancellationToken,
    adapters: Vec<Arc<dyn orka_core::traits::ChannelAdapter>>,
    queue: Arc<dyn PriorityQueue>,
    gateway_handle: JoinHandle<()>,
    worker_handle: JoinHandle<()>,
    outbound_handle: JoinHandle<()>,
) -> anyhow::Result<()> {
    let _ = sd_notify::notify(true, &[sd_notify::NotifyState::Ready]);
    info!(
        listen = %format_args!("{}:{}", config.server.host, config.server.port),
        "Orka server ready",
    );
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("failed to register SIGTERM handler")?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => info!("received SIGINT"),
        _ = sigterm.recv() => info!("received SIGTERM"),
    }

    let _ = sd_notify::notify(false, &[sd_notify::NotifyState::Stopping]);
    info!("shutting down...");
    for adapter in &adapters {
        if let Err(e) = adapter.shutdown().await {
            error!(%e, "adapter shutdown error");
        }
    }

    // Drain queue with timeout
    let drain_timeout = std::time::Duration::from_secs(30);
    let drain_start = std::time::Instant::now();
    loop {
        match queue.len().await {
            Ok(0) => {
                info!("queue drained");
                break;
            }
            Ok(n) => {
                if drain_start.elapsed() >= drain_timeout {
                    warn!(remaining = n, "drain timeout reached, forcing shutdown");
                    break;
                }
                tokio::time::sleep(std::time::Duration::from_millis(200)).await;
            }
            Err(_) => break,
        }
    }

    shutdown.cancel();

    let (gw, wk, ob) = tokio::join!(gateway_handle, worker_handle, outbound_handle);
    if let Err(e) = gw {
        warn!(%e, "gateway task failed");
    }
    if let Err(e) = wk {
        warn!(%e, "worker task failed");
    }
    if let Err(e) = ob {
        warn!(%e, "outbound task failed");
    }
    info!("Orka server stopped");

    Ok(())
}

// ---------------------------------------------------------------------------
// Bootstrap lifecycle
// ---------------------------------------------------------------------------

async fn init_commands(
    skill_bundle: &SkillBundle,
    infra: &InfraBundle,
    workspace_registry: &Arc<WorkspaceRegistry>,
    config: &OrkaConfig,
    experience_service: Option<Arc<orka_experience::ExperienceService>>,
    adapters: &[Arc<dyn orka_core::traits::ChannelAdapter>],
) -> anyhow::Result<Arc<CommandRegistry>> {
    let mut commands = CommandRegistry::new();
    let cmd_deps = orka_worker::commands::CommandRegistryDeps {
        skills: skill_bundle.skills.clone(),
        memory: infra.memory.clone(),
        facts: skill_bundle.fact_store.clone(),
        secrets: infra.secrets.clone(),
        workspace_registry: workspace_registry.clone(),
        agent_config: config
            .agents
            .first()
            .context("no agents defined after configuration validation")?
            .config
            .clone(),
        experience: experience_service,
    };
    orka_worker::commands::register_all(&mut commands, cmd_deps);
    let commands = Arc::new(commands);
    let cmd_list = commands.list();
    for adapter in adapters {
        if let Err(e) = adapter.register_commands(&cmd_list).await {
            warn!(%e, channel = adapter.channel_id(), "failed to register commands with adapter");
        }
    }
    Ok(commands)
}

impl Bootstrap {
    /// Run all initialization phases and return the fully-wired server state.
    async fn new() -> anyhow::Result<Self> {
        let config = init_config_and_tracing()?;
        let (infra, metrics_handle) = init_infra(&config)?;

        // Migrate any plaintext secrets to encrypted format. No-op when no
        // encryption key is set.
        let migrated = infra
            .secrets
            .migrate_plaintext_secrets()
            .await
            .context("failed to migrate plaintext secrets")?;
        if migrated > 0 {
            info!(migrated, "plaintext secrets migrated to encrypted");
        }

        let skill_bundle = init_skills(&config).await?;

        // 5. LLM clients
        let llm = build_llm_clients(&config, &*infra.secrets).await;
        if llm.client.is_some() {
            info!("LLM client ready");
        } else {
            error!(
                "no LLM providers initialized — set ANTHROPIC_API_KEY, MOONSHOT_API_KEY, or OPENAI_API_KEY to enable AI responses"
            );
        }
        let env_watcher = crate::env_watcher::EnvWatcher::start(
            config.llm.providers.clone(),
            config.llm.default_model.clone(),
            llm.swappable,
            infra.secrets.clone(),
        );
        let guardrail = orka_guardrails::create_guardrail(&config.guardrails);
        if guardrail.is_some() {
            info!("guardrails enabled");
        }

        // 6. Load workspace(s) + start file watchers
        let workspace_registry = load_workspaces(&config).await?;
        let shutdown = CancellationToken::new();
        let (auth_enabled, auth_layer) = build_auth_layer(&config);

        // 7. Start all adapters
        let stream_registry = orka_core::StreamRegistry::new();
        let mobile_events = MobileEventHub::new();
        let (_custom_adapter, adapters) = start_all_adapters(AdapterStartArgs {
            secrets: infra.secrets.clone(),
            bus: infra.bus.clone(),
            shutdown: shutdown.clone(),
            memory: infra.memory.clone(),
            auth_layer: auth_layer.clone(),
            stream_registry: stream_registry.clone(),
            config: config.clone(),
        })
        .await?;
        let start_time = std::time::Instant::now();

        // 8. Gateway
        let gateway_handle = spawn_gateway(&infra, &workspace_registry, &config, shutdown.clone())?;

        // 9. Experience / self-learning service
        let experience_service = if config.experience.enabled {
            match create_experience_service(&config) {
                Ok(svc) => svc,
                Err(e) => {
                    warn!(%e, "failed to initialize experience service");
                    None
                }
            }
        } else {
            None
        };

        // 10. Agent graph
        let graph = Arc::new(
            orka_agent::build_graph_from_config(
                &config.agents,
                config.graph.as_ref(),
                &config.llm,
                &workspace_registry,
            )
            .await
            .context("failed to build agent graph")?,
        );
        info!(graph_id = %graph.id, "agent graph built");

        // 11. Command registry + register with adapters
        let commands = init_commands(
            &skill_bundle,
            &infra,
            &workspace_registry,
            &config,
            experience_service.clone(),
            &adapters,
        )
        .await?;

        Ok(Self {
            config,
            infra,
            metrics_handle,
            skill_bundle,
            llm_client: llm.client,
            guardrail,
            workspace_registry,
            shutdown,
            adapters,
            experience_service,
            graph,
            commands,
            gateway_handle,
            start_time,
            stream_registry,
            mobile_events,
            auth_enabled,
            _env_watcher: env_watcher,
        })
    }

    /// Start the execution layer and run until shutdown.
    async fn run(self) -> anyhow::Result<()> {
        // 12. A2A state
        let agent_directory = Arc::new(orka_a2a::AgentDirectory::new());
        let a2a_state =
            build_a2a_state(&self.config, &self.skill_bundle.skills, &self.infra.secrets);
        spawn_a2a_discovery(&self.config, agent_directory.clone(), self.shutdown.clone());

        let checkpoint_store = build_checkpoint_store(&self.config.redis.url);

        // Create shared cancel tokens upfront so both the HTTP router (cancel
        // endpoint) and the worker pool share the same map.
        let session_cancel_tokens: orka_worker::SessionCancelTokens =
            std::sync::Arc::new(std::sync::Mutex::new(std::collections::HashMap::new()));

        start_http_server(HttpServerDeps {
            config: &self.config,
            infra: &self.infra,
            metrics_handle: self.metrics_handle,
            skill_bundle: &self.skill_bundle,
            experience_service: &self.experience_service,
            workspace_registry: &self.workspace_registry,
            graph: &self.graph,
            adapters: &self.adapters,
            a2a_state,
            agent_directory,
            checkpoint_store: checkpoint_store.clone(),
            start_time: self.start_time,
            auth_enabled: self.auth_enabled,
            stream_registry: self.stream_registry.clone(),
            mobile_events: self.mobile_events.clone(),
            session_cancel_tokens: session_cancel_tokens.clone(),
        })
        .await?;

        // 13. Template registry from default workspace
        let lock = self
            .workspace_registry
            .default_state()
            .context("default workspace not registered")?;
        let templates = lock.read().await.templates.clone();

        // 14. Executor + dispatcher + worker pool
        let memory_for_worker = Arc::clone(&self.infra.memory);
        let memory_lock = Arc::clone(&self.infra.memory_lock);
        let executor = Arc::new(orka_agent::GraphExecutor::new(orka_agent::ExecutorDeps {
            skills: self.skill_bundle.skills.clone(),
            memory: self.infra.memory.clone(),
            secrets: self.infra.secrets.clone(),
            llm: self.llm_client,
            event_sink: self.infra.event_sink.clone(),
            stream_registry: self.stream_registry,
            experience: self.experience_service.clone(),
            facts: self.skill_bundle.fact_store,
            soft_skills: self.skill_bundle.soft_skills,
            templates,
            coding_runtime: self.skill_bundle.coding_runtime,
            guardrail: self.guardrail,
            checkpoint_store,
            bus: Some(self.infra.bus.clone()),
        }));
        let dispatcher = Arc::new(GraphDispatcher::new(
            executor,
            self.graph,
            Some(memory_for_worker),
            Some(self.commands.clone()),
        ));
        let worker_handle = spawn_worker_pool(
            &self.config,
            &self.infra,
            dispatcher,
            memory_lock,
            self.shutdown.clone(),
            session_cancel_tokens,
        );

        // 15. Scheduler loop
        let _scheduler_handle = spawn_scheduler_loop(
            &self.config,
            self.skill_bundle.scheduler_store,
            &self.skill_bundle.skills,
            &self.infra.event_sink,
            self.shutdown.clone(),
        );

        // 16. Distillation loop
        let _distillation_handle = spawn_distillation_loop(
            self.experience_service.clone(),
            &self.workspace_registry,
            &self.infra.event_sink,
            self.config.experience.distillation_interval_secs,
            self.shutdown.clone(),
        );

        // 17. Outbound bridge
        let outbound_handle = spawn_outbound_bridge(
            &self.infra.bus,
            self.infra.conversations.clone(),
            self.infra.artifacts.clone(),
            self.mobile_events.clone(),
            self.adapters.clone(),
            self.shutdown.clone(),
        )
        .await?;

        // 18–19. Ready signal + graceful shutdown
        wait_for_shutdown(
            &self.config,
            self.shutdown,
            self.adapters,
            self.infra.queue.clone(),
            self.gateway_handle,
            worker_handle,
            outbound_handle,
        )
        .await
    }
}

// ---------------------------------------------------------------------------
// Main entry point
// ---------------------------------------------------------------------------

/// Run the Orka server until SIGINT or SIGTERM.
pub(crate) async fn run() -> anyhow::Result<()> {
    Bootstrap::new().await?.run().await
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use std::sync::Arc;

    use orka_core::{
        Conversation, ConversationId, Envelope, MessageId, SessionId,
        testing::InMemoryConversationStore, traits::ConversationStore,
    };

    use super::*;

    #[tokio::test]
    async fn mobile_outbound_error_sets_failed_state_and_emits_failure() {
        let artifacts = Arc::new(orka_core::testing::InMemoryArtifactStore::new());
        let store = Arc::new(InMemoryConversationStore::new());
        let mobile_events = MobileEventHub::new();
        let conversation_id = ConversationId::new();
        let session_id = SessionId::from(conversation_id);
        let conversation = Conversation::new(conversation_id, session_id, "user-1", "Test");
        store.put_conversation(&conversation).await.unwrap();

        let mut envelope = Envelope::text("mobile", session_id, "tool execution failed");
        envelope.id = MessageId::new();
        envelope.metadata.insert(
            "stop_reason".into(),
            serde_json::json!(AgentStopReason::Error),
        );

        let mut events = mobile_events.subscribe(conversation_id).await;
        persist_mobile_outbound(
            store.as_ref(),
            artifacts.as_ref(),
            &mobile_events,
            &envelope,
        )
        .await
        .unwrap();

        let updated = store
            .get_conversation(&conversation_id)
            .await
            .unwrap()
            .expect("conversation should exist");
        assert_eq!(updated.status, ConversationStatus::Failed);

        let messages = store
            .list_messages(&conversation_id, None, None, usize::MAX)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].status, ConversationMessageStatus::Failed);
        assert_eq!(messages[0].text, "tool execution failed");

        let event = events
            .recv()
            .await
            .expect("failure event should be emitted");
        match event {
            orka_contracts::RealtimeEvent::MessageFailed {
                conversation_id: event_conversation_id,
                error,
            } => {
                assert_eq!(event_conversation_id, conversation_id.as_uuid());
                assert_eq!(error, "tool execution failed");
            }
            other => panic!("unexpected mobile event: {other:?}"),
        }
    }

    #[tokio::test]
    async fn mobile_outbound_interrupted_sets_interrupted_state_and_emits_completed_message() {
        let artifacts = Arc::new(orka_core::testing::InMemoryArtifactStore::new());
        let store = Arc::new(InMemoryConversationStore::new());
        let mobile_events = MobileEventHub::new();
        let conversation_id = ConversationId::new();
        let session_id = SessionId::from(conversation_id);
        let conversation = Conversation::new(conversation_id, session_id, "user-1", "Test");
        store.put_conversation(&conversation).await.unwrap();

        let mut envelope = Envelope::text("mobile", session_id, "Paused for approval.");
        envelope.id = MessageId::new();
        envelope.metadata.insert(
            "stop_reason".into(),
            serde_json::json!(AgentStopReason::Interrupted),
        );

        let mut events = mobile_events.subscribe(conversation_id).await;
        persist_mobile_outbound(
            store.as_ref(),
            artifacts.as_ref(),
            &mobile_events,
            &envelope,
        )
        .await
        .unwrap();

        let updated = store
            .get_conversation(&conversation_id)
            .await
            .unwrap()
            .expect("conversation should exist");
        assert_eq!(updated.status, ConversationStatus::Interrupted);

        let messages = store
            .list_messages(&conversation_id, None, None, usize::MAX)
            .await
            .unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].status, ConversationMessageStatus::Completed);
        assert_eq!(messages[0].text, "Paused for approval.");

        let event = events
            .recv()
            .await
            .expect("completion event should be emitted");
        match event {
            orka_contracts::RealtimeEvent::MessageCompleted { message, .. } => {
                assert_eq!(
                    message["id"].as_str(),
                    Some(envelope.id.to_string().as_str())
                );
                assert_eq!(message["text"].as_str(), Some("Paused for approval."));
            }
            other => panic!("unexpected mobile event: {other:?}"),
        }
    }

    // --- select_coding_backend ---

    fn coding_cfg(default_provider: CodingProvider, policy: CodingSelectionPolicy) -> CodingConfig {
        let mut cfg = CodingConfig::default();
        cfg.default_provider = default_provider;
        cfg.selection_policy = policy;
        cfg
    }

    #[test]
    fn select_coding_claude_available() {
        let cfg = coding_cfg(
            CodingProvider::ClaudeCode,
            CodingSelectionPolicy::Availability,
        );
        assert_eq!(
            select_coding_backend(&cfg, true, false, false),
            Some(CodingProvider::ClaudeCode)
        );
    }

    #[test]
    fn select_coding_claude_unavailable_returns_none() {
        let cfg = coding_cfg(
            CodingProvider::ClaudeCode,
            CodingSelectionPolicy::Availability,
        );
        assert_eq!(select_coding_backend(&cfg, false, true, true), None);
    }

    #[test]
    fn select_coding_codex_explicit() {
        let cfg = coding_cfg(CodingProvider::Codex, CodingSelectionPolicy::Availability);
        assert_eq!(
            select_coding_backend(&cfg, false, true, false),
            Some(CodingProvider::Codex)
        );
    }

    #[test]
    fn select_coding_auto_availability_prefers_claude_then_codex() {
        let cfg = coding_cfg(CodingProvider::Auto, CodingSelectionPolicy::Availability);
        // claude available: picks claude
        assert_eq!(
            select_coding_backend(&cfg, true, true, true),
            Some(CodingProvider::ClaudeCode)
        );
        // claude not available, codex available: picks codex
        assert_eq!(
            select_coding_backend(&cfg, false, true, true),
            Some(CodingProvider::Codex)
        );
        // only opencode
        assert_eq!(
            select_coding_backend(&cfg, false, false, true),
            Some(CodingProvider::OpenCode)
        );
        // nothing available
        assert_eq!(select_coding_backend(&cfg, false, false, false), None);
    }

    #[test]
    fn select_coding_auto_prefer_codex_picks_codex_first() {
        let cfg = coding_cfg(CodingProvider::Auto, CodingSelectionPolicy::PreferCodex);
        assert_eq!(
            select_coding_backend(&cfg, true, true, false),
            Some(CodingProvider::Codex)
        );
        // codex unavailable falls back to claude
        assert_eq!(
            select_coding_backend(&cfg, true, false, false),
            Some(CodingProvider::ClaudeCode)
        );
    }

    #[test]
    fn select_coding_auto_prefer_opencode_picks_opencode_first() {
        let cfg = coding_cfg(CodingProvider::Auto, CodingSelectionPolicy::PreferOpenCode);
        assert_eq!(
            select_coding_backend(&cfg, true, true, true),
            Some(CodingProvider::OpenCode)
        );
    }

    // --- to_runtime_observe_config ---

    #[test]
    fn to_runtime_observe_maps_all_fields() {
        let mut src = orka_config::ObserveConfig::default();
        src.enabled = true;
        src.backend = "prometheus".to_string();
        src.otlp_endpoint = Some("http://otel:4317".to_string());
        src.batch_size = 42;
        src.flush_interval_ms = 500;
        src.service_name = "my-service".to_string();
        src.service_version = "1.2.3".to_string();
        let dst = to_runtime_observe_config(&src);
        assert!(dst.enabled);
        assert_eq!(dst.backend, "prometheus");
        assert_eq!(dst.otlp_endpoint.as_deref(), Some("http://otel:4317"));
        assert_eq!(dst.batch_size, 42);
        assert_eq!(dst.flush_interval_ms, 500);
        assert_eq!(dst.service_name, "my-service");
        assert_eq!(dst.service_version, "1.2.3");
    }

    // --- to_runtime_audit_config ---

    #[test]
    fn to_runtime_audit_maps_all_fields() {
        let mut src = orka_config::AuditConfig::default();
        src.enabled = true;
        src.output = "redis".to_string();
        src.path = Some("/tmp/audit.log".into());
        src.redis_key = Some("orka:audit".to_string());
        let dst = to_runtime_audit_config(&src);
        assert!(dst.enabled);
        assert_eq!(dst.output, "redis");
        assert_eq!(
            dst.path.as_deref(),
            Some(std::path::Path::new("/tmp/audit.log"))
        );
        assert_eq!(dst.redis_key.as_deref(), Some("orka:audit"));
    }

    // --- to_runtime_memory_config ---

    #[test]
    fn to_runtime_memory_maps_max_entries() {
        let mut src = orka_config::MemoryConfig::default();
        src.max_entries = 250;
        let dst = to_runtime_memory_config(&src);
        assert_eq!(dst.max_entries, 250);
    }

    #[test]
    fn to_runtime_memory_maps_redis_backend() {
        let mut src = orka_config::MemoryConfig::default();
        src.backend = MemoryBackend::Redis;
        let dst = to_runtime_memory_config(&src);
        assert!(matches!(
            dst.backend,
            orka_memory::config::MemoryBackend::Redis
        ));
    }

    #[test]
    fn to_runtime_memory_maps_memory_backend() {
        let mut src = orka_config::MemoryConfig::default();
        src.backend = MemoryBackend::Memory;
        let dst = to_runtime_memory_config(&src);
        assert!(matches!(
            dst.backend,
            orka_memory::config::MemoryBackend::Memory
        ));
    }

    // --- to_runtime_secret_config ---

    #[test]
    fn to_runtime_secret_maps_file_path() {
        let mut src = orka_config::SecretConfig::default();
        src.file_path = Some("/etc/orka/secrets.json".to_string());
        let dst = to_runtime_secret_config(&src);
        assert_eq!(dst.file_path.as_deref(), Some("/etc/orka/secrets.json"));
    }

    #[test]
    fn to_runtime_secret_maps_encryption_key_path() {
        let mut src = orka_config::SecretConfig::default();
        src.encryption_key_path = Some("/etc/orka/key.pem".to_string());
        let dst = to_runtime_secret_config(&src);
        assert_eq!(
            dst.encryption_key_path.as_deref(),
            Some("/etc/orka/key.pem")
        );
    }

    // --- to_runtime_sandbox_config ---

    #[test]
    fn to_runtime_sandbox_maps_backend_and_limits() {
        let mut src = orka_config::SandboxConfig::default();
        src.backend = "wasmtime".to_string();
        src.limits.timeout_secs = 10;
        src.limits.max_memory_bytes = 64 * 1024 * 1024;
        let dst = to_runtime_sandbox_config(&src);
        assert_eq!(dst.backend, "wasmtime");
        assert_eq!(dst.limits.timeout_secs, 10);
        assert_eq!(dst.limits.max_memory_bytes, 64 * 1024 * 1024);
    }

    #[test]
    fn to_runtime_sandbox_maps_allowed_denied_paths() {
        let mut src = orka_config::SandboxConfig::default();
        src.allowed_paths = vec!["/tmp".to_string(), "/data".to_string()];
        src.denied_paths = vec!["/etc".to_string()];
        let dst = to_runtime_sandbox_config(&src);
        assert_eq!(dst.allowed_paths, vec!["/tmp", "/data"]);
        assert_eq!(dst.denied_paths, vec!["/etc"]);
    }
}
