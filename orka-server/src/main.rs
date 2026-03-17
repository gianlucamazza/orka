mod env_watcher;

#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

use std::collections::HashMap;
use std::future::IntoFuture;
use std::sync::Arc;

use anyhow::Context;
use orka_adapter_custom::CustomAdapter;
use orka_bus::create_bus;
use orka_core::config::{LlmProviderConfig, OrkaConfig};
use orka_core::traits::{ChannelAdapter, SecretManager};
use orka_core::{Envelope, OutboundMessage, Payload};
use orka_gateway::Gateway;
use orka_llm::SwappableLlmClient;
use orka_queue::create_queue;
use orka_session::create_session_store;
use orka_worker::{CommandRegistry, WorkerPool, WorkspaceHandler, WorkspaceHandlerConfig};
use orka_workspace::{WorkspaceLoader, WorkspaceRegistry};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tower_http::limit::RequestBodyLimitLayer;
use tracing::{error, info, warn};
use tracing_subscriber::EnvFilter;
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

/// Maximum request body size for server API endpoints: 1 MB.
const MAX_BODY_SIZE: usize = 1024 * 1024;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Orka API",
        description = "Orka AI Agent Orchestration Platform",
        version = "0.1.0"
    ),
    paths(
        orka_adapter_custom::routes::handle_message,
        orka_adapter_custom::routes::handle_health,
    ),
    components(schemas(
        orka_core::Envelope,
        orka_core::Payload,
        orka_core::Priority,
        orka_core::OutboundMessage,
        orka_core::Session,
        orka_core::SessionId,
        orka_core::MessageId,
        orka_core::TraceContext,
        orka_core::MediaPayload,
        orka_core::CommandPayload,
        orka_core::EventPayload,
    )),
    tags(
        (name = "messages", description = "Message endpoints"),
        (name = "health", description = "Health check endpoints")
    )
)]
struct ApiDoc;

/// Middleware that adds security headers to all responses.
async fn security_headers(
    req: axum::http::Request<axum::body::Body>,
    next: axum::middleware::Next,
) -> axum::response::Response {
    let mut response = next.run(req).await;
    let headers = response.headers_mut();
    headers.insert(
        http::header::X_CONTENT_TYPE_OPTIONS,
        http::HeaderValue::from_static("nosniff"),
    );
    headers.insert(
        http::header::X_FRAME_OPTIONS,
        http::HeaderValue::from_static("DENY"),
    );
    headers.insert(
        http::header::STRICT_TRANSPORT_SECURITY,
        http::HeaderValue::from_static("max-age=63072000; includeSubDomains"),
    );
    headers.insert(
        http::HeaderName::from_static("x-content-security-policy"),
        http::HeaderValue::from_static("default-src 'none'"),
    );
    response
}

/// Start an adapter: create inbound bridge (adapter sink → bus "inbound")
/// and return the adapter Arc.
///
/// If `workspace_name` is provided, it's injected as `workspace:name` metadata
/// on every inbound envelope so the worker can resolve the correct workspace.
async fn start_adapter(
    adapter: Arc<dyn ChannelAdapter>,
    bus: Arc<dyn orka_core::traits::MessageBus>,
    shutdown: CancellationToken,
    workspace_name: Option<String>,
) -> anyhow::Result<()> {
    let (sink_tx, mut sink_rx) = mpsc::channel::<Envelope>(256);
    adapter.start(sink_tx).await?;

    let bus_for_bridge = bus.clone();
    let cancel = shutdown.clone();
    tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                msg = sink_rx.recv() => {
                    match msg {
                        Some(mut envelope) => {
                            if let Some(ref ws) = workspace_name {
                                envelope.metadata.entry("workspace:name".to_string())
                                    .or_insert_with(|| serde_json::json!(ws));
                            }
                            if let Err(e) = bus_for_bridge.publish("inbound", &envelope).await {
                                error!(%e, "failed to publish inbound envelope to bus");
                            }
                        }
                        None => break,
                    }
                }
            }
        }
    });
    Ok(())
}

/// Adapter to bridge orka_skills::SkillRegistry with orka_scheduler::SkillRegistry trait.
struct SchedulerSkillRegistryAdapter(Arc<orka_skills::SkillRegistry>);

#[async_trait::async_trait]
impl orka_scheduler::SkillRegistry for SchedulerSkillRegistryAdapter {
    async fn invoke(
        &self,
        name: &str,
        input: orka_core::SkillInput,
    ) -> orka_core::Result<orka_core::SkillOutput> {
        self.0.invoke(name, input).await
    }
}

/// Default environment variable name for a provider's API key.
fn default_env_var(provider: &str) -> &str {
    match provider {
        "anthropic" => "ANTHROPIC_API_KEY",
        "openai" => "OPENAI_API_KEY",
        _ => "",
    }
}

/// Resolve an API key for a provider using a 4-level fallback:
///   1. `api_key` in config (direct)
///   2. `api_key_env` (explicit env var name from config)
///   3. Default env var (e.g. `ANTHROPIC_API_KEY`)
///   4. Secret store (`api_key_secret`)
pub(crate) async fn resolve_api_key(
    provider: &str,
    config: &LlmProviderConfig,
    secrets: &dyn SecretManager,
) -> Option<String> {
    // 1. Direct API key in config
    let key = config.api_key.clone().filter(|k| !k.is_empty());
    // 2. Explicit env var name
    let key = key.or_else(|| {
        config
            .api_key_env
            .as_deref()
            .and_then(|env| std::env::var(env).ok().filter(|k| !k.is_empty()))
    });
    // 3. Default env var
    let default_env = default_env_var(provider);
    let key = key.or_else(|| {
        if default_env.is_empty() {
            return None;
        }
        std::env::var(default_env).ok().filter(|k| !k.is_empty())
    });
    // 4. Secret store
    let key = if key.is_some() {
        key
    } else if let Some(key_name) = config.api_key_secret.as_deref() {
        match secrets.get_secret(key_name).await {
            Ok(s) => {
                let k = s.expose_str().unwrap_or("").to_string();
                if k.is_empty() {
                    tracing::debug!(provider, path = key_name, "secret exists but is empty");
                    None
                } else {
                    Some(k)
                }
            }
            Err(e) => {
                tracing::debug!(provider, path = key_name, %e, "failed to read secret from store");
                None
            }
        }
    } else {
        None
    };

    if key.is_none() {
        let env_name = if !default_env.is_empty() {
            default_env
        } else {
            "N/A"
        };
        warn!(
            provider,
            "API key not found in config, secrets, or {env_name} env var"
        );
    }
    key
}
#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // 0. Load .env file (no-op if missing — production uses systemd EnvironmentFile)
    let _ = dotenvy::dotenv();

    // 1. Load config (doesn't need tracing)
    let mut config = OrkaConfig::load(None).context("failed to load configuration")?;
    config
        .validate()
        .context("configuration validation failed")?;

    // 2. Init tracing from config (RUST_LOG takes precedence)
    let filter =
        EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new(&config.logging.level));
    if config.logging.json {
        tracing_subscriber::fmt()
            .with_env_filter(filter)
            .json()
            .init();
    } else {
        tracing_subscriber::fmt().with_env_filter(filter).init();
    }
    info!(?config, "Orka server starting");

    // 3. Create infra
    let bus = create_bus(&config).context("failed to create message bus")?;
    let sessions = create_session_store(&config).context("failed to create session store")?;
    let queue = create_queue(&config).context("failed to create priority queue")?;

    let memory =
        orka_memory::create_memory_store(&config).context("failed to create memory store")?;
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

    // 4c. Load WASM plugins
    if let Some(ref plugin_dir) = config.plugins.dir {
        match orka_skills::load_plugins(std::path::Path::new(plugin_dir)) {
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

    // 4d. MCP servers
    for server_config in &config.mcp.servers {
        let mcp_config = orka_mcp::McpServerConfig {
            name: server_config.name.clone(),
            command: server_config.command.clone(),
            args: server_config.args.clone(),
            env: server_config.env.clone(),
        };
        match orka_mcp::McpClient::connect(mcp_config).await {
            Ok(client) => {
                let client = Arc::new(client);
                match client.list_tools().await {
                    Ok(tools) => {
                        for tool in tools {
                            let bridge = orka_mcp::McpToolBridge::new(
                                client.clone(),
                                tool.name.clone(),
                                tool.description.unwrap_or_default(),
                                tool.input_schema,
                            );
                            skills.register(Arc::new(bridge));
                            info!(tool = %tool.name, server = %server_config.name, "registered MCP tool");
                        }
                    }
                    Err(e) => warn!(%e, server = %server_config.name, "failed to list MCP tools"),
                }
            }
            Err(e) => warn!(%e, server = %server_config.name, "failed to connect to MCP server"),
        }
    }

    // 4e. Web skills (web_search + web_read)
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
    if config.http.enabled {
        match orka_http::create_http_skills(&config.http) {
            Ok(http_skills) => {
                for skill in http_skills {
                    skills.register(skill);
                }
            }
            Err(e) => warn!(%e, "failed to initialize HTTP skills"),
        }
    }

    // 4g. Knowledge/RAG skills
    if config.knowledge.enabled {
        match orka_knowledge::create_knowledge_skills(&config.knowledge) {
            Ok(knowledge_skills) => {
                for skill in knowledge_skills {
                    skills.register(skill);
                }
            }
            Err(e) => warn!(%e, "failed to initialize knowledge skills"),
        }
    }

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
    if config.os.enabled {
        match orka_os::create_os_skills(&config.os) {
            Ok(os_skills) => {
                for skill in os_skills {
                    skills.register(skill);
                }
            }
            Err(e) => warn!(%e, "failed to initialize OS skills"),
        }
    }

    let skills = Arc::new(skills);
    info!("skill registry ready ({} skills)", skills.list().len());

    // LLM client (optional) — after validate(), config.llm.providers is canonical
    // Track swappable clients for hot-reload
    let mut swappable_clients: HashMap<String, Arc<SwappableLlmClient>> = HashMap::new();
    let llm_client: Option<Arc<dyn orka_llm::LlmClient>> = if !config.llm.providers.is_empty() {
        let mut clients: Vec<(String, Arc<dyn orka_llm::LlmClient>, Vec<String>)> = Vec::new();
        for pc in &config.llm.providers {
            let client: Option<Arc<dyn orka_llm::LlmClient>> = match pc.provider.as_str() {
                "anthropic" => {
                    let key = resolve_api_key("anthropic", pc, &*secrets).await;
                    key.map(|k| {
                        Arc::new(orka_llm::AnthropicClient::with_options(
                            k,
                            pc.model.clone(),
                            pc.timeout_secs.unwrap_or(30),
                            pc.max_tokens.unwrap_or(8192),
                            pc.max_retries.unwrap_or(2),
                            config.llm.api_version.clone(),
                        )) as Arc<dyn orka_llm::LlmClient>
                    })
                }
                "openai" => {
                    let key = resolve_api_key("openai", pc, &*secrets).await;
                    key.map(|k| {
                        let url = pc
                            .base_url
                            .clone()
                            .unwrap_or_else(|| "https://api.openai.com/v1".into());
                        Arc::new(orka_llm::OpenAiClient::with_options(
                            k,
                            pc.model.clone(),
                            pc.timeout_secs.unwrap_or(30),
                            pc.max_tokens.unwrap_or(8192),
                            pc.max_retries.unwrap_or(2),
                            url,
                        )) as Arc<dyn orka_llm::LlmClient>
                    })
                }
                "ollama" => {
                    let url = pc
                        .base_url
                        .clone()
                        .unwrap_or_else(|| "http://localhost:11434/v1".into());
                    Some(Arc::new(orka_llm::OllamaClient::with_options(
                        pc.model.clone(),
                        pc.timeout_secs.unwrap_or(30),
                        pc.max_tokens.unwrap_or(8192),
                        pc.max_retries.unwrap_or(2),
                        url,
                    )) as Arc<dyn orka_llm::LlmClient>)
                }
                other => {
                    warn!(provider = other, "unknown LLM provider");
                    None
                }
            };
            if let Some(c) = client {
                info!(provider = %pc.name, model = %pc.model, "LLM provider initialized");
                // Wrap in SwappableLlmClient for hot-reload support
                let swappable = Arc::new(SwappableLlmClient::new(c));
                swappable_clients.insert(pc.name.clone(), swappable.clone());
                clients.push((
                    pc.name.clone(),
                    swappable as Arc<dyn orka_llm::LlmClient>,
                    pc.prefixes.clone(),
                ));
            }
        }
        if clients.is_empty() {
            None
        } else if clients.len() == 1 {
            Some(clients.remove(0).1)
        } else {
            let (_, default_client, _) = clients.remove(0);
            let mut router = orka_llm::LlmRouter::new(default_client);
            for (name, client, prefixes) in clients {
                router = router.add_provider(name, client, prefixes);
            }
            Some(Arc::new(router) as Arc<dyn orka_llm::LlmClient>)
        }
    } else {
        None
    };

    if llm_client.is_some() {
        info!("LLM client ready");
    } else {
        error!(
            "no LLM providers initialized — set ANTHROPIC_API_KEY or OPENAI_API_KEY to enable AI responses"
        );
    }

    // Start env file watcher for API key hot-reload
    let _env_watcher = env_watcher::EnvWatcher::start(
        config.llm.providers.clone(),
        swappable_clients,
        secrets.clone(),
        config.llm.api_version.clone(),
    );

    // Guardrails
    let guardrail = orka_guardrails::create_guardrail(&config.guardrails);
    if guardrail.is_some() {
        info!("guardrails enabled");
    }

    // 5. Load workspace(s) into registry
    let workspace_registry = if config.workspaces.is_empty() {
        // Backward-compatible: single workspace from workspace_dir
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
        for entry in &config.workspaces {
            let loader = Arc::new(WorkspaceLoader::new(&entry.dir));
            loader.load_all().await?;
            info!(workspace = %entry.name, dir = %entry.dir, "workspace loaded");
            reg.register(entry.name.clone(), loader);
        }
        reg
    };
    let workspace_registry = Arc::new(workspace_registry);

    // 5a. Start workspace watchers for all registered workspaces
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

    // Shutdown token (created early so heartbeat can use it)
    let shutdown = CancellationToken::new();

    // 5b. Heartbeat task (if configured)
    if let Some(interval_secs) = config.agent.heartbeat_interval_secs {
        let event_sink_hb = event_sink.clone();
        let hb_shutdown = shutdown.clone();
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(interval_secs));
            loop {
                tokio::select! {
                    _ = hb_shutdown.cancelled() => break,
                    _ = interval.tick() => {
                        event_sink_hb.emit(orka_core::DomainEvent {
                            id: orka_core::EventId::new(),
                            timestamp: chrono::Utc::now(),
                            kind: orka_core::DomainEventKind::Heartbeat,
                            metadata: Default::default(),
                        }).await;
                    }
                }
            }
        });
        info!(interval_secs, "heartbeat task started");
    }

    // 5c. Auth
    let auth_layer = if config.auth.enabled {
        use orka_auth::{ApiKeyAuthenticator, AuthLayer};
        let authenticator = ApiKeyAuthenticator::new(&config.auth.api_keys);
        Some(AuthLayer::new(
            Arc::new(authenticator),
            Arc::new(config.auth.clone()),
        ))
    } else {
        None
    };

    // Stream registry (shared between worker handler and custom adapter)
    let stream_registry = orka_core::StreamRegistry::new();

    // 6. Create + start adapters
    let mut adapters: Vec<Arc<dyn ChannelAdapter>> = Vec::new();

    // 6a. Custom adapter (always started)
    let adapter_config = config.adapters.custom.clone().unwrap_or_default();
    let custom_adapter: Arc<dyn ChannelAdapter> = Arc::new(CustomAdapter::new(
        adapter_config,
        auth_layer,
        stream_registry.clone(),
    ));
    let custom_ws = config
        .adapters
        .custom
        .as_ref()
        .and_then(|c| c.workspace.clone());
    start_adapter(
        custom_adapter.clone(),
        bus.clone(),
        shutdown.clone(),
        custom_ws,
    )
    .await?;
    adapters.push(custom_adapter);
    info!("custom adapter started");

    // 6b. Telegram adapter (optional)
    if let Some(ref tg_config) = config.adapters.telegram {
        let secret_name = tg_config
            .bot_token_secret
            .as_deref()
            .unwrap_or("telegram_bot_token");
        match secrets.get_secret(secret_name).await {
            Ok(secret) => {
                let token = secret.expose_str().unwrap_or("").to_string();
                if !token.is_empty() {
                    let tg: Arc<dyn ChannelAdapter> =
                        Arc::new(orka_adapter_telegram::TelegramAdapter::new(token));
                    start_adapter(
                        tg.clone(),
                        bus.clone(),
                        shutdown.clone(),
                        tg_config.workspace.clone(),
                    )
                    .await?;
                    adapters.push(tg);
                    info!("telegram adapter started");
                } else {
                    warn!("telegram bot token is empty, adapter disabled");
                }
            }
            Err(e) => warn!(%e, "failed to load telegram bot token, adapter disabled"),
        }
    }

    // 6c. Discord adapter (optional)
    if let Some(ref dc_config) = config.adapters.discord {
        let secret_name = dc_config
            .bot_token_secret
            .as_deref()
            .unwrap_or("discord_bot_token");
        match secrets.get_secret(secret_name).await {
            Ok(secret) => {
                let token = secret.expose_str().unwrap_or("").to_string();
                if !token.is_empty() {
                    let dc: Arc<dyn ChannelAdapter> =
                        Arc::new(orka_adapter_discord::DiscordAdapter::new(token));
                    start_adapter(
                        dc.clone(),
                        bus.clone(),
                        shutdown.clone(),
                        dc_config.workspace.clone(),
                    )
                    .await?;
                    adapters.push(dc);
                    info!("discord adapter started");
                } else {
                    warn!("discord bot token is empty, adapter disabled");
                }
            }
            Err(e) => warn!(%e, "failed to load discord bot token, adapter disabled"),
        }
    }

    // 6d. Slack adapter (optional)
    if let Some(ref slack_config) = config.adapters.slack {
        let secret_name = slack_config
            .bot_token_secret
            .as_deref()
            .unwrap_or("slack_bot_token");
        match secrets.get_secret(secret_name).await {
            Ok(secret) => {
                let token = secret.expose_str().unwrap_or("").to_string();
                if !token.is_empty() {
                    let slack: Arc<dyn ChannelAdapter> = Arc::new(
                        orka_adapter_slack::SlackAdapter::new(token, slack_config.listen_port),
                    );
                    start_adapter(
                        slack.clone(),
                        bus.clone(),
                        shutdown.clone(),
                        slack_config.workspace.clone(),
                    )
                    .await?;
                    adapters.push(slack);
                    info!(port = slack_config.listen_port, "slack adapter started");
                } else {
                    warn!("slack bot token is empty, adapter disabled");
                }
            }
            Err(e) => warn!(%e, "failed to load slack bot token, adapter disabled"),
        }
    }

    // 6e. WhatsApp adapter (optional)
    if let Some(ref wa_config) = config.adapters.whatsapp {
        let access_secret = wa_config
            .access_token_secret
            .as_deref()
            .unwrap_or("whatsapp_access_token");
        let verify_secret = wa_config
            .verify_token_secret
            .as_deref()
            .unwrap_or("whatsapp_verify_token");
        let phone_id = wa_config.phone_number_id.clone().unwrap_or_default();

        match (
            secrets.get_secret(access_secret).await,
            secrets.get_secret(verify_secret).await,
        ) {
            (Ok(access), Ok(verify)) => {
                let access_token = access.expose_str().unwrap_or("").to_string();
                let verify_token = verify.expose_str().unwrap_or("").to_string();
                if !access_token.is_empty() && !phone_id.is_empty() {
                    let wa: Arc<dyn ChannelAdapter> =
                        Arc::new(orka_adapter_whatsapp::WhatsAppAdapter::new(
                            access_token,
                            phone_id,
                            verify_token,
                            wa_config.listen_port,
                        ));
                    start_adapter(
                        wa.clone(),
                        bus.clone(),
                        shutdown.clone(),
                        wa_config.workspace.clone(),
                    )
                    .await?;
                    adapters.push(wa);
                    info!(port = wa_config.listen_port, "whatsapp adapter started");
                } else {
                    warn!("whatsapp access token or phone_number_id is empty, adapter disabled");
                }
            }
            _ => warn!("failed to load whatsapp secrets, adapter disabled"),
        }
    }

    // 6f. Health + API endpoints on server port
    let start_time = std::time::Instant::now();
    let queue_for_health = queue.clone();
    let config_concurrency = config.worker.concurrency;

    let queue_for_dlq = queue.clone();
    let mut public_routes = axum::Router::new();

    // /metrics endpoint (Prometheus)
    if let Some(handle) = metrics_handle {
        let handle = Arc::new(handle);
        public_routes = public_routes.route(
            "/metrics",
            axum::routing::get(move || {
                let h = handle.clone();
                async move { h.render() }
            }),
        );
    }

    let public_routes = public_routes
        .route(
            "/health",
            axum::routing::get(move || {
                let queue = queue_for_health.clone();
                async move {
                    let uptime_secs = start_time.elapsed().as_secs();
                    let queue_depth = queue.len().await.unwrap_or(0);

                    axum::Json(serde_json::json!({
                        "status": "ok",
                        "uptime_secs": uptime_secs,
                        "workers": config_concurrency,
                        "queue_depth": queue_depth,
                    }))
                }
            }),
        )
        .route(
            "/health/live",
            axum::routing::get(|| async { axum::Json(serde_json::json!({"status": "ok"})) }),
        )
        .route(
            "/health/ready",
            axum::routing::get({
                let queue = queue.clone();
                let redis_url = config.redis.url.clone();
                move || {
                    let queue = queue.clone();
                    let redis_url = redis_url.clone();
                    async move {
                        let mut checks = serde_json::Map::new();
                        let mut all_ok = true;

                        // Redis check
                        match redis::Client::open(redis_url.as_str()) {
                            Ok(client) => match client.get_multiplexed_async_connection().await {
                                Ok(mut conn) => {
                                    match redis::cmd("PING").query_async::<String>(&mut conn).await
                                    {
                                        Ok(_) => {
                                            checks.insert("redis".into(), serde_json::json!("ok"));
                                        }
                                        Err(e) => {
                                            checks.insert(
                                                "redis".into(),
                                                serde_json::json!(format!("error: {e}")),
                                            );
                                            all_ok = false;
                                        }
                                    }
                                }
                                Err(e) => {
                                    checks.insert(
                                        "redis".into(),
                                        serde_json::json!(format!("error: {e}")),
                                    );
                                    all_ok = false;
                                }
                            },
                            Err(e) => {
                                checks.insert(
                                    "redis".into(),
                                    serde_json::json!(format!("error: {e}")),
                                );
                                all_ok = false;
                            }
                        }

                        // Queue check
                        match queue.len().await {
                            Ok(depth) => {
                                checks.insert(
                                    "queue".into(),
                                    serde_json::json!({"status": "ok", "depth": depth}),
                                );
                            }
                            Err(e) => {
                                checks.insert(
                                    "queue".into(),
                                    serde_json::json!(format!("error: {e}")),
                                );
                                all_ok = false;
                            }
                        }

                        let status = if all_ok { "ready" } else { "not_ready" };
                        let code = if all_ok {
                            axum::http::StatusCode::OK
                        } else {
                            axum::http::StatusCode::SERVICE_UNAVAILABLE
                        };

                        (
                            code,
                            axum::Json(serde_json::json!({
                                "status": status,
                                "checks": checks,
                            })),
                        )
                    }
                }
            }),
        );

    // Protected API routes — auth middleware applied when enabled
    let api_routes = axum::Router::new()
        .route(
            "/api/v1/dlq",
            axum::routing::get({
                let q = queue_for_dlq.clone();
                move || {
                    let q = q.clone();
                    async move {
                        match q.list_dlq().await {
                            Ok(items) => {
                                let json: Vec<serde_json::Value> = items
                                    .iter()
                                    .map(|e| {
                                        serde_json::json!({
                                            "id": e.id.to_string(),
                                            "channel": e.channel,
                                            "session_id": e.session_id.to_string(),
                                            "timestamp": e.timestamp.to_rfc3339(),
                                            "metadata": e.metadata,
                                        })
                                    })
                                    .collect();
                                axum::response::IntoResponse::into_response(axum::Json(json))
                            }
                            Err(e) => axum::response::IntoResponse::into_response((
                                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                format!("DLQ list failed: {e}"),
                            )),
                        }
                    }
                }
            })
            .delete({
                let q = queue_for_dlq.clone();
                move || {
                    let q = q.clone();
                    async move {
                        match q.purge_dlq().await {
                            Ok(count) => axum::response::IntoResponse::into_response(axum::Json(
                                serde_json::json!({ "purged": count }),
                            )),
                            Err(e) => axum::response::IntoResponse::into_response((
                                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                format!("DLQ purge failed: {e}"),
                            )),
                        }
                    }
                }
            }),
        )
        .route(
            "/api/v1/dlq/{id}/replay",
            axum::routing::post({
                let q = queue_for_dlq.clone();
                move |axum::extract::Path(id): axum::extract::Path<String>| {
                    let q = q.clone();
                    async move {
                        let msg_id = match uuid::Uuid::parse_str(&id) {
                            Ok(uuid) => orka_core::MessageId(uuid),
                            Err(_) => {
                                return axum::response::IntoResponse::into_response((
                                    axum::http::StatusCode::BAD_REQUEST,
                                    "invalid message ID",
                                ));
                            }
                        };
                        match q.replay_dlq(&msg_id).await {
                            Ok(true) => axum::response::IntoResponse::into_response(axum::Json(
                                serde_json::json!({ "replayed": true }),
                            )),
                            Ok(false) => axum::response::IntoResponse::into_response((
                                axum::http::StatusCode::NOT_FOUND,
                                "message not found in DLQ",
                            )),
                            Err(e) => axum::response::IntoResponse::into_response((
                                axum::http::StatusCode::INTERNAL_SERVER_ERROR,
                                format!("DLQ replay failed: {e}"),
                            )),
                        }
                    }
                }
            }),
        );

    // Build auth layer for API routes
    let api_auth_layer = if config.auth.enabled {
        use orka_auth::{ApiKeyAuthenticator, AuthLayer};
        let authenticator = ApiKeyAuthenticator::new(&config.auth.api_keys);
        Some(AuthLayer::new(
            Arc::new(authenticator),
            Arc::new(config.auth.clone()),
        ))
    } else {
        None
    };

    let api_routes = if let Some(layer) = api_auth_layer {
        axum::Router::new().merge(api_routes.layer(layer))
    } else {
        api_routes
    };

    // A2A protocol routes (if enabled)
    let public_routes = if config.a2a.enabled {
        let base_url = config
            .a2a
            .url
            .clone()
            .unwrap_or_else(|| format!("http://{}:{}", config.server.host, config.server.port));
        let agent_card =
            orka_a2a::build_agent_card("orka", "Orka AI Agent Platform", &base_url, &skills);
        let a2a_state = orka_a2a::A2aState {
            agent_card,
            skills: skills.clone(),
            secrets: secrets.clone(),
            tasks: Default::default(),
        };
        public_routes.merge(orka_a2a::a2a_router(a2a_state))
    } else {
        public_routes
    };

    let health_app = public_routes
        .merge(api_routes)
        .merge(SwaggerUi::new("/docs").url("/api-doc/openapi.json", ApiDoc::openapi()))
        .layer(axum::middleware::from_fn(security_headers))
        .layer(RequestBodyLimitLayer::new(MAX_BODY_SIZE));
    let listener =
        tokio::net::TcpListener::bind(format!("{}:{}", config.server.host, config.server.port))
            .await
            .context("failed to bind health endpoint")?;
    info!(
        "health endpoint listening on {}:{}",
        config.server.host, config.server.port
    );
    tokio::spawn(axum::serve(listener, health_app).into_future());

    // 7. Start gateway (with config)
    let gateway = Gateway::new(
        bus.clone(),
        sessions.clone(),
        queue.clone(),
        workspace_registry.default_loader().clone(),
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

    // 8. Build command registry and start worker pool with WorkspaceHandler
    let mut commands = CommandRegistry::new();
    orka_worker::commands::register_all(
        &mut commands,
        skills.clone(),
        memory.clone(),
        secrets.clone(),
        workspace_registry.clone(),
        &config.agent,
    );
    let commands = Arc::new(commands);

    let disabled_tools: std::collections::HashSet<String> =
        config.tools.disabled.iter().cloned().collect();

    let handler: Arc<dyn orka_worker::AgentHandler> = Arc::new(WorkspaceHandler::new(
        workspace_registry.clone(),
        skills.clone(),
        memory,
        secrets,
        llm_client,
        event_sink.clone(),
        WorkspaceHandlerConfig {
            agent_config: config.agent.clone(),
            disabled_tools,
            default_context_window: config.llm.context_window_tokens,
        },
        guardrail,
        commands,
        stream_registry.clone(),
    ));
    let worker_pool = WorkerPool::new(
        queue.clone(),
        sessions.clone(),
        bus.clone(),
        handler,
        event_sink.clone(),
        config.worker.concurrency,
        config.queue.max_retries,
    )
    .with_retry_delay(config.worker.retry_base_delay_ms);
    let worker_cancel = shutdown.clone();
    let worker_handle = tokio::spawn(async move {
        if let Err(e) = worker_pool.run(worker_cancel).await {
            error!(%e, "worker pool error");
        }
    });

    // 8b. Start scheduler loop (if enabled)
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

    // 9. Outbound bridge: bus "outbound" → route to correct adapter by channel
    let mut outbound_rx = bus.subscribe("outbound").await?;
    let adapters_out = adapters.clone();
    let outbound_cancel = shutdown.clone();
    let outbound_handle = tokio::spawn(async move {
        loop {
            tokio::select! {
                _ = outbound_cancel.cancelled() => break,
                msg = outbound_rx.recv() => {
                    match msg {
                        Some(envelope) => {
                            let text = match &envelope.payload {
                                Payload::Text(t) => t.clone(),
                                _ => "[non-text]".into(),
                            };
                            let outbound = OutboundMessage {
                                channel: envelope.channel.clone(),
                                session_id: envelope.session_id.clone(),
                                payload: Payload::Text(text),
                                reply_to: None,
                                metadata: envelope.metadata.clone(),
                            };
                            // Route to the adapter whose channel_id matches
                            let target = adapters_out.iter().find(|a| a.channel_id() == envelope.channel.as_str());
                            if let Some(adapter) = target {
                                if let Err(e) = adapter.send(outbound).await {
                                    error!(%e, channel = %envelope.channel, "failed to send outbound message via adapter");
                                }
                            } else {
                                warn!(channel = %envelope.channel, "no adapter found for outbound channel");
                            }
                        }
                        None => {
                            warn!("outbound bus channel closed");
                            break;
                        }
                    }
                }
            }
        }
    });

    // 10. Await ctrl-c or SIGTERM → graceful shutdown
    info!("Orka server ready — press Ctrl+C to stop");
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .context("failed to register SIGTERM handler")?;
    tokio::select! {
        _ = tokio::signal::ctrl_c() => info!("received SIGINT"),
        _ = sigterm.recv() => info!("received SIGTERM"),
    }

    info!("shutting down...");
    for adapter in &adapters {
        if let Err(e) = adapter.shutdown().await {
            error!(%e, "adapter shutdown error");
        }
    }

    // Graceful drain: wait for queue to empty (with timeout)
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

    let _ = tokio::join!(gateway_handle, worker_handle, outbound_handle);
    info!("Orka server stopped");

    Ok(())
}
