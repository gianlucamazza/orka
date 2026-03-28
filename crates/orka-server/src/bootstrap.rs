//! Server bootstrap: orchestrates all initialization steps and runs until
//! shutdown.

use std::{future::IntoFuture, sync::Arc};

use anyhow::Context;
use orka_bus::create_bus;
use orka_core::{OutboundMessage, config::OrkaConfig};
use orka_gateway::Gateway;
use orka_queue::{QueueBundle, create_queue};
use orka_server::router::{
    BUILD_DATE, GIT_SHA, RouterParams, ServerFeatures, VERSION, build_router,
};
use orka_session::create_session_store;
use orka_worker::{CommandRegistry, GraphDispatcher, WorkerPool};
use orka_workspace::{WorkspaceLoader, WorkspaceRegistry};
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

fn select_coding_backend(
    config: &orka_core::config::CodingConfig,
    claude_available: bool,
    codex_available: bool,
    opencode_available: bool,
) -> Option<orka_core::config::CodingProvider> {
    use orka_core::config::{CodingProvider, CodingSelectionPolicy};
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

/// Run the Orka server until SIGINT or SIGTERM.
pub(crate) async fn run() -> anyhow::Result<()> {
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

    // 3. Create infra
    let bus = create_bus(&config).context("failed to create message bus")?;
    let sessions = create_session_store(&config).context("failed to create session store")?;
    let QueueBundle { queue, dlq } =
        create_queue(&config).context("failed to create priority queue")?;
    let orka_memory::MemoryBundle {
        store: memory,
        lock: memory_lock,
    } = orka_memory::create_memory_store(&config).context("failed to create memory store")?;
    info!("memory store ready");
    let secrets =
        orka_secrets::create_secret_manager(&config).context("failed to create secret manager")?;
    info!("secret manager ready");
    let event_sink = orka_observe::create_event_sink(&config);
    info!("event sink ready");

    // 3b. Install Prometheus metrics recorder
    let metrics_handle = orka_observe::metrics::install_prometheus_recorder();
    if metrics_handle.is_some() {
        info!("prometheus metrics recorder installed");
    }

    // 4. Skill registry
    let mut skills = orka_skills::create_skill_registry();
    skills.register(Arc::new(orka_skills::EchoSkill));

    // 4b. Sandbox + SandboxSkill
    let sandbox =
        orka_sandbox::create_sandbox(&config.sandbox).context("failed to create sandbox")?;
    skills.register(Arc::new(orka_sandbox::SandboxSkill::new(sandbox)));

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

    // 4d. MCP servers (connect in parallel)
    let mcp_server_count = {
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
        let mut mcp_server_count: usize = 0;
        while let Some(result) = mcp_set.join_next().await {
            match result {
                Ok(Ok((server_name, client, tools))) => {
                    mcp_server_count += 1;
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
        mcp_server_count
    };

    // 4e. Web skills
    if config.web.search_provider != "none" {
        let web_config = orka_web::WebConfig {
            search_provider: match config.web.search_provider.as_str() {
                "tavily" => orka_web::SearchProviderKind::Tavily,
                "brave" => orka_web::SearchProviderKind::Brave,
                "searxng" => orka_web::SearchProviderKind::Searxng,
                _ => orka_web::SearchProviderKind::None,
            },
            api_key: config.web.api_key.clone(),
            api_key_env: config.web.api_key_env.clone(),
            searxng_base_url: config.web.searxng_base_url.clone(),
            max_results: config.web.max_results,
            max_read_chars: config.web.max_read_chars,
            max_content_chars: config.web.max_content_chars,
            cache_ttl_secs: config.web.cache_ttl_secs,
            read_timeout_secs: config.web.read_timeout_secs,
            user_agent: config.web.user_agent.clone(),
        };
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
    match orka_http::create_http_skills(&config.http) {
        Ok(http_skills) => {
            for skill in http_skills {
                skills.register(skill);
            }
        }
        Err(e) => warn!(%e, "failed to initialize HTTP skills"),
    }

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

    // 4i. OS skills
    let mut coding_runtime = None;
    if config.os.enabled {
        let caps = orka_os::EnvironmentCapabilities::probe(&config.os).await;
        let claude_code_available =
            config.os.coding.providers.claude_code.enabled && caps.claude_code.available;
        let codex_available = config.os.coding.providers.codex.enabled && caps.codex.available;
        let opencode_available =
            config.os.coding.providers.opencode.enabled && caps.opencode.available;
        let selected_backend = select_coding_backend(
            &config.os.coding,
            claude_code_available,
            codex_available,
            opencode_available,
        );
        use orka_core::config::CodingProvider;
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
        coding_runtime = Some(orka_agent::executor::CodingRuntimeStatus {
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
        });
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
    }

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
                    skills.register(std::sync::Arc::from(skill));
                }
                info!(skill_count = count, "git skills initialized");
            }
            Err(e) => warn!(%e, "failed to initialize git skills"),
        }
    }

    let skills = Arc::new(skills);
    info!("skill registry ready ({} skills)", skills.list().len());

    // 4j. Soft skills (SKILL.md-based instruction skills)
    let soft_skills: Option<Arc<orka_skills::SoftSkillRegistry>> = if let Some(ref dir) =
        config.soft_skills.dir
    {
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
    } else {
        None
    };

    // 5. LLM clients
    let llm = build_llm_clients(&config, &*secrets).await;
    if llm.client.is_some() {
        info!("LLM client ready");
    } else {
        error!(
            "no LLM providers initialized — set ANTHROPIC_API_KEY, MOONSHOT_API_KEY, or OPENAI_API_KEY to enable AI responses"
        );
    }

    // Start env file watcher for API key hot-reload
    let _env_watcher = crate::env_watcher::EnvWatcher::start(
        config.llm.providers.clone(),
        config.llm.default_model.clone(),
        llm.swappable,
        secrets.clone(),
    );

    // Guardrails
    let guardrail = orka_guardrails::create_guardrail(&config.guardrails);
    if guardrail.is_some() {
        info!("guardrails enabled");
    }

    // 6. Load workspace(s) into registry
    let workspace_registry = if config.workspaces.is_empty() {
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
    let workspace_registry = Arc::new(workspace_registry);

    // 6a. Start workspace file watchers
    let mut _watchers = Vec::new();
    for ws_name in workspace_registry.list_names() {
        if let Some(loader) = workspace_registry.get(ws_name) {
            match orka_workspace::WorkspaceWatcher::start(loader.clone()) {
                Ok(w) => _watchers.push(w),
                Err(e) => warn!(workspace = %ws_name, %e, "failed to start workspace watcher"),
            }
        }
    }
    info!("workspace watchers started");

    let shutdown = CancellationToken::new();

    // 6c. Auth layer
    let auth_enabled = config.auth.jwt.is_some() || !config.auth.api_keys.is_empty();
    let auth_layer = if auth_enabled {
        use orka_auth::{ApiKeyAuthenticator, AuthLayer, middleware::AuthMiddlewareConfig};
        let authenticator = ApiKeyAuthenticator::new(&config.auth.api_keys);
        Some(AuthLayer::new(
            Arc::new(authenticator),
            Arc::new(AuthMiddlewareConfig::default()),
        ))
    } else {
        None
    };

    // 7. Start all adapters
    let stream_registry = orka_core::StreamRegistry::new();
    let (_custom_adapter, adapters) = start_all_adapters(AdapterStartArgs {
        secrets: secrets.clone(),
        bus: bus.clone(),
        shutdown: shutdown.clone(),
        memory: memory.clone(),
        auth_layer: auth_layer.clone(),
        stream_registry: stream_registry.clone(),
        config: config.clone(),
    })
    .await?;

    let start_time = std::time::Instant::now();

    // 8. Gateway
    let gateway = Gateway::new(
        bus.clone(),
        sessions.clone(),
        queue.clone(),
        workspace_registry
            .default_loader()
            .context("default workspace not registered")?
            .clone(),
        event_sink.clone(),
        Some(&config.redis.url),
        config.gateway.rate_limit,
        config.gateway.dedup_ttl_secs,
    );
    let gateway_cancel = shutdown.clone();
    let gateway_handle = tokio::spawn(async move {
        if let Err(e) = gateway.run(gateway_cancel).await {
            error!(%e, "gateway error");
        }
    });

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
        orka_agent::build_graph_from_config(&config, &workspace_registry)
            .await
            .context("failed to build agent graph")?,
    );
    info!(graph_id = %graph.id, "agent graph built");

    // 11. Command registry + register with adapters
    let mut commands = CommandRegistry::new();
    orka_worker::commands::register_all(
        &mut commands,
        skills.clone(),
        memory.clone(),
        fact_store.clone(),
        secrets.clone(),
        workspace_registry.clone(),
        &config
            .agents
            .first()
            .context("no agents defined after configuration validation")?
            .config,
        experience_service.clone(),
    );
    let commands = Arc::new(commands);
    {
        let cmd_list = commands.list();
        for adapter in &adapters {
            if let Err(e) = adapter.register_commands(&cmd_list).await {
                warn!(%e, channel = adapter.channel_id(), "failed to register commands with adapter");
            }
        }
    }

    // 12. Build HTTP management + health router
    let auth_layer_for_router = if auth_enabled {
        let authenticator = orka_auth::ApiKeyAuthenticator::new(&config.auth.api_keys);
        Some(orka_auth::AuthLayer::new(
            Arc::new(authenticator),
            Arc::new(orka_auth::middleware::AuthMiddlewareConfig::default()),
        ))
    } else {
        None
    };

    let a2a_enabled = config.a2a.discovery_enabled || !config.a2a.known_agents.is_empty();
    let agent_directory = Arc::new(orka_a2a::AgentDirectory::new());
    let a2a_state = if a2a_enabled {
        let base_url = format!("http://{}:{}", config.server.host, config.server.port);
        let agent_card = orka_a2a::build_agent_card_with_auth(
            "orka",
            "Orka AI Agent Platform",
            &base_url,
            &skills,
            config.a2a.auth_enabled.then_some(&config.auth),
        );

        let use_redis = config.a2a.store_backend == "redis";
        let (task_store, push_store): (
            Arc<dyn orka_a2a::TaskStore>,
            Arc<dyn orka_a2a::PushNotificationStore>,
        ) = if use_redis {
            let redis_url = &config.redis.url;
            let ts = orka_a2a::RedisTaskStore::new(redis_url)
                .map(|s| Arc::new(s) as Arc<dyn orka_a2a::TaskStore>)
                .unwrap_or_else(|e| {
                    tracing::warn!(%e, "failed to create RedisTaskStore, falling back to memory");
                    Arc::new(orka_a2a::InMemoryTaskStore::default())
                });
            let ps = orka_a2a::RedisPushNotificationStore::new(redis_url)
                .map(|s| Arc::new(s) as Arc<dyn orka_a2a::PushNotificationStore>)
                .unwrap_or_else(|e| {
                    tracing::warn!(%e, "failed to create RedisPushNotificationStore, falling back to memory");
                    Arc::new(orka_a2a::InMemoryPushNotificationStore::default())
                });
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
            task_events: Default::default(),
            push_store: push_store.clone(),
            webhook_deliverer: Arc::new(orka_a2a::WebhookDeliverer::new(push_store)),
        })
    } else {
        None
    };

    // Spawn A2A discovery client if known agent URLs are configured.
    if !config.a2a.known_agents.is_empty() {
        let client = orka_a2a::DiscoveryClient::new(
            config.a2a.known_agents.clone(),
            config.a2a.discovery_interval_secs,
            agent_directory.clone(),
        );
        tokio::spawn(client.run(shutdown.clone()));
    }

    let qdrant_url = if config.knowledge.enabled {
        config.knowledge.vector_store.url.clone()
    } else {
        None
    };

    let entry = graph.entry_agent();
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

    let agent_count = graph.nodes_iter().count();

    let adapter_names: Vec<String> = adapters
        .iter()
        .map(|a| a.channel_id().to_string())
        .filter(|id| id != "custom") // internal transport, not a user-facing channel
        .collect();

    let coding_backend = coding_runtime
        .as_ref()
        .and_then(|r| r.selected_backend.clone());

    let web_search = if config.web.search_provider == "none" {
        None
    } else {
        Some(config.web.search_provider.clone())
    };

    let features = ServerFeatures {
        knowledge: config.knowledge.enabled,
        scheduler: config.scheduler.enabled,
        experience: config.experience.enabled,
        guardrails: config.guardrails.enabled,
        a2a: a2a_enabled,
        observe: config.observe.enabled,
        research: config.research.enabled,
    };

    // Build checkpoint store from existing Redis config. Errors are logged and
    // the server falls back to no checkpointing rather than aborting startup.
    let checkpoint_store: Option<Arc<dyn orka_checkpoint::CheckpointStore>> =
        match orka_checkpoint::RedisCheckpointStore::new_default_ttl(&config.redis.url) {
            Ok(store) => {
                info!("checkpoint store: Redis at {}", config.redis.url);
                Some(Arc::new(store))
            }
            Err(e) => {
                warn!(%e, "checkpoint store unavailable, running without persistence");
                None
            }
        };

    let health_app = build_router(RouterParams {
        queue: queue.clone(),
        dlq: dlq.clone(),
        skills: skills.clone(),
        soft_skills: soft_skills.clone(),
        sessions: sessions.clone(),
        scheduler_store: scheduler_store.clone(),
        checkpoint_store: checkpoint_store.clone(),
        workspace_registry: workspace_registry.clone(),
        graph: graph.clone(),
        experience_service: experience_service.clone(),
        start_time,
        concurrency: config.worker.concurrency,
        redis_url: config.redis.url.clone(),
        qdrant_url,
        auth_layer: auth_layer_for_router,
        a2a_state,
        a2a_auth_enabled: config.a2a.auth_enabled,
        agent_directory,
        metrics_handle,
        agent_name,
        agent_model,
        mcp_server_count,
        features,
        thinking,
        agent_count,
        auth_enabled,
        adapters: adapter_names,
        coding_backend,
        web_search,
        research_service: None,
        stream_registry: orka_core::StreamRegistry::new(),
    });

    let listener =
        tokio::net::TcpListener::bind(format!("{}:{}", config.server.host, config.server.port))
            .await
            .context("failed to bind health endpoint")?;
    info!(
        "health endpoint listening on {}:{}",
        config.server.host, config.server.port
    );
    tokio::spawn(axum::serve(listener, health_app).into_future());

    // 13. Get template registry from workspace
    let templates = {
        let default_state = workspace_registry
            .default_state()
            .context("default workspace not registered")?;
        let state = default_state.read().await;
        state.templates.clone()
    };

    // 14. Executor + worker pool
    let memory_for_worker = memory.clone();
    let memory_lock_for_worker = memory_lock.clone();

    let executor = Arc::new(orka_agent::GraphExecutor::new(orka_agent::ExecutorDeps {
        skills: skills.clone(),
        memory,
        secrets,
        llm: llm.client,
        event_sink: event_sink.clone(),
        stream_registry,
        experience: experience_service.clone(),
        facts: fact_store,
        soft_skills,
        templates,
        coding_runtime,
        guardrail,
        checkpoint_store,
        bus: Some(bus.clone()),
    }));

    let dispatcher = Arc::new(GraphDispatcher::new(
        executor,
        graph,
        Some(memory_for_worker),
        Some(commands.clone()),
    ));
    let worker_pool = WorkerPool::new(
        queue.clone(),
        sessions.clone(),
        bus.clone(),
        dispatcher,
        event_sink.clone(),
        config.worker.concurrency,
        config.queue.max_retries,
    )
    .with_retry_delay(config.worker.retry_base_delay_ms)
    .with_session_lock(memory_lock_for_worker)
    .with_dlq(dlq.clone());
    let worker_cancel = shutdown.clone();
    let worker_handle = tokio::spawn(async move {
        if let Err(e) = worker_pool.run(worker_cancel).await {
            error!(%e, "worker pool error");
        }
    });

    // 15. Scheduler loop
    let _scheduler_handle = if let Some(store) = scheduler_store {
        let scheduler = orka_scheduler::Scheduler::new(
            store,
            Arc::new(SchedulerSkillRegistryAdapter(skills.clone())),
            config.scheduler.poll_interval_secs,
            config.scheduler.max_concurrent,
        );
        let scheduler_cancel = shutdown.clone();
        Some(tokio::spawn(async move {
            scheduler.run(scheduler_cancel).await;
        }))
    } else {
        None
    };

    // 16. Distillation loop
    let _distillation_handle = if let Some(ref exp) = experience_service {
        let interval_secs = config.experience.distillation_interval_secs;
        if interval_secs > 0 {
            let exp = exp.clone();
            let workspace_names: Vec<String> = workspace_registry
                .list_names()
                .into_iter()
                .map(std::string::ToString::to_string)
                .collect();
            let distill_event_sink = event_sink.clone();
            let distill_cancel = shutdown.clone();
            Some(tokio::spawn(async move {
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(interval_secs));
                interval.tick().await;
                loop {
                    tokio::select! {
                        () = distill_cancel.cancelled() => break,
                        _ = interval.tick() => {
                            for ws in &workspace_names {
                                match exp.distill(ws).await {
                                    Ok(count) if count > 0 => {
                                        info!(workspace = %ws, principles_created = count, "distillation completed");
                                        distill_event_sink.emit(orka_core::DomainEvent::new(
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
        } else {
            None
        }
    } else {
        None
    };

    // 17. Outbound bridge: bus "outbound" → route to correct adapter by channel
    let mut outbound_rx = bus.subscribe("outbound").await?;
    let adapters_out = adapters.clone();
    let outbound_cancel = shutdown.clone();
    let outbound_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                () = outbound_cancel.cancelled() => break,
                msg = outbound_rx.recv() => {
                    if let Some(envelope) = msg {
                        let mut outbound = OutboundMessage::new(
                            envelope.channel.clone(),
                            envelope.session_id,
                            envelope.payload.clone(),
                            None,
                        );
                        outbound.metadata = envelope.metadata.clone();
                        let target = adapters_out
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
    });

    // 18. Signal ready + wait for shutdown
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

    // 19. Graceful shutdown
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
