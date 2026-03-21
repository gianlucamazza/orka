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
use orka_experience::ExperienceService;
use orka_gateway::Gateway;
use orka_llm::SwappableLlmClient;
use orka_queue::create_queue;
use orka_session::create_session_store;
use orka_worker::{CommandRegistry, WorkerPoolGraph};
use orka_workspace::{WorkspaceLoader, WorkspaceRegistry};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tower_http::limit::RequestBodyLimitLayer;
use tracing::{debug, error, info, warn};
use tracing_subscriber::EnvFilter;

const VERSION: &str = env!("CARGO_PKG_VERSION");
const GIT_SHA: &str = env!("ORKA_GIT_SHA");
const BUILD_DATE: &str = env!("ORKA_BUILD_DATE");

const GITHUB_LATEST_URL: &str = "https://api.github.com/repos/gianlucamazza/orka/releases/latest";

/// Spawn a fire-and-forget task that checks for a newer release on GitHub
/// and logs a warning if one exists. Never blocks startup.
fn spawn_update_check() {
    tokio::spawn(async {
        let client = match reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(10))
            .user_agent(format!("orka-server/{VERSION}"))
            .build()
        {
            Ok(c) => c,
            Err(_) => return,
        };
        let Ok(resp) = client
            .get(GITHUB_LATEST_URL)
            .header("Accept", "application/vnd.github.v3+json")
            .send()
            .await
        else {
            return;
        };
        let Ok(json) = resp.json::<serde_json::Value>().await else {
            return;
        };
        let Some(tag) = json["tag_name"].as_str() else {
            return;
        };
        let latest = tag.trim_start_matches('v');
        if semver_gt(latest, VERSION) {
            let upgrade_hint = upgrade_hint_for_server();
            warn!(
                current = VERSION,
                latest, upgrade_hint, "A new version of orka-server is available."
            );
        }
    });
}

/// Returns an upgrade hint string based on the server binary's install location.
fn upgrade_hint_for_server() -> &'static str {
    let is_docker = std::path::Path::new("/.dockerenv").exists()
        || std::fs::read_to_string("/proc/1/cgroup")
            .map(|c| c.contains("docker") || c.contains("containerd"))
            .unwrap_or(false);
    if is_docker {
        return "Pull the latest image and recreate the container.";
    }
    let exe = match std::env::current_exe() {
        Ok(p) => p,
        Err(_) => return "See https://github.com/gianlucamazza/orka/releases",
    };
    let s = exe.to_string_lossy();
    if s.starts_with("/usr/bin/") {
        "Run: yay -Syu orka-git"
    } else if s.starts_with("/usr/local/bin/") {
        "Run: orka update  (or re-run install.sh)"
    } else if s.contains("/.cargo/bin/") {
        "Run: cargo install orka"
    } else {
        "See https://github.com/gianlucamazza/orka/releases"
    }
}

/// Returns true if `a` is a higher semver than `b` (major.minor.patch only).
fn semver_gt(a: &str, b: &str) -> bool {
    let parse = |v: &str| -> (u64, u64, u64) {
        let mut it = v.split('.').filter_map(|p| p.parse::<u64>().ok());
        (
            it.next().unwrap_or(0),
            it.next().unwrap_or(0),
            it.next().unwrap_or(0),
        )
    };
    parse(a) > parse(b)
}
use utoipa::OpenApi;
use utoipa_swagger_ui::SwaggerUi;

/// Maximum request body size for server API endpoints: 1 MB.
const MAX_BODY_SIZE: usize = 1024 * 1024;

#[derive(OpenApi)]
#[openapi(
    info(
        title = "Orka API",
        description = "Orka AI Agent Orchestration Platform",
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

/// Create the experience / self-learning service from config.
///
/// Reuses the knowledge config for embedding provider and vector store settings.
/// Returns `None` if experience is disabled or initialization fails.
fn create_experience_service(
    config: &OrkaConfig,
) -> anyhow::Result<Option<Arc<ExperienceService>>> {
    use orka_knowledge::embeddings::EmbeddingProvider;
    use orka_knowledge::vector_store::VectorStore;

    // We need an LLM client for reflection — create a lightweight one from the first provider
    let first_provider = config
        .llm
        .providers
        .first()
        .ok_or_else(|| anyhow::anyhow!("experience requires at least one LLM provider"))?;

    let api_key = first_provider
        .api_key
        .clone()
        .or_else(|| {
            first_provider
                .api_key_env
                .as_ref()
                .and_then(|env| std::env::var(env).ok())
        })
        .or_else(|| std::env::var(default_env_var(&first_provider.provider)).ok())
        .ok_or_else(|| {
            anyhow::anyhow!(
                "experience reflection requires an API key for provider '{}'",
                first_provider.name
            )
        })?;

    let model = config
        .experience
        .reflection_model
        .clone()
        .unwrap_or_else(|| first_provider.model.clone());

    let reflection_llm: Arc<dyn orka_llm::LlmClient> = match first_provider.provider.as_str() {
        "openai" => Arc::new(orka_llm::OpenAiClient::new(api_key, model)),
        "ollama" => Arc::new(orka_llm::OllamaClient::new(model)),
        _ => Arc::new(orka_llm::AnthropicClient::with_options(
            api_key,
            model,
            first_provider
                .timeout_secs
                .unwrap_or(config.llm.timeout_secs),
            first_provider.max_tokens.unwrap_or(config.llm.max_tokens),
            first_provider.max_retries.unwrap_or(config.llm.max_retries),
            config.llm.api_version.clone(),
            first_provider.base_url.clone(),
        )),
    };

    // Create embedding provider (reusing knowledge config)
    let embedding_provider: Arc<dyn EmbeddingProvider> =
        match config.knowledge.embeddings.provider.as_str() {
            "openai" => {
                let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
                    anyhow::anyhow!("OPENAI_API_KEY required for openai embedding provider")
                })?;
                Arc::new(
                    orka_knowledge::embeddings::openai::OpenAiEmbeddingProvider::new(
                        api_key,
                        config.knowledge.embeddings.model.clone(),
                        config.knowledge.embeddings.dimensions,
                    ),
                )
            }
            _ => Arc::new(
                orka_knowledge::embeddings::local::LocalEmbeddingProvider::new(
                    &config.knowledge.embeddings.model,
                    config.knowledge.embeddings.dimensions,
                )
                .map_err(|e| anyhow::anyhow!("failed to create local embedding provider: {e}"))?,
            ),
        };

    // Create vector store
    let vector_store: Arc<dyn VectorStore> = Arc::new(
        orka_knowledge::vector_store::qdrant::QdrantStore::new(&config.knowledge.vector_store.url)
            .map_err(|e| anyhow::anyhow!("failed to create Qdrant store: {e}"))?,
    );

    let service = orka_experience::create_experience_service(
        &config.experience,
        embedding_provider,
        vector_store,
        reflection_llm,
    )
    .map_err(|e| anyhow::anyhow!("failed to create experience service: {e}"))?;

    Ok(service)
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

    // 1b. Ensure the workspace state directory is in os.allowed_paths so
    // PermissionGuard permits access regardless of deployment layout
    // (e.g. /var/lib/orka under systemd ProtectSystem=strict).
    if config.os.enabled {
        let workspace_parent = std::path::Path::new(&config.workspace_dir)
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| config.workspace_dir.clone());
        if !config
            .os
            .allowed_paths
            .iter()
            .any(|p| workspace_parent.starts_with(p.as_str()))
        {
            config.os.allowed_paths.push(workspace_parent);
        }
    }

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

    // 4b. Shared WASM engine + Sandbox + SandboxSkill
    let wasm_engine =
        orka_wasm::WasmEngine::new().context("failed to create shared WASM engine")?;
    let sandbox =
        orka_sandbox::create_sandbox(&config.sandbox).context("failed to create sandbox")?;
    skills.register(Arc::new(orka_sandbox::SandboxSkill::new(sandbox)));

    // 4c. Load WASM plugins
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

    // 4d. MCP servers (connect in parallel)
    {
        let mut mcp_set = tokio::task::JoinSet::new();
        for server_config in &config.mcp.servers {
            let transport = match (&server_config.command, &server_config.url) {
                (Some(cmd), _) => orka_mcp::McpTransportConfig::Stdio {
                    command: cmd.clone(),
                    args: server_config.args.clone(),
                    env: server_config.env.clone(),
                },
                (None, Some(url)) => orka_mcp::McpTransportConfig::StreamableHttp {
                    url: url.clone(),
                    auth: server_config
                        .auth
                        .as_ref()
                        .map(|a| orka_mcp::McpOAuthConfig {
                            token_url: a.token_url.clone(),
                            client_id: a.client_id.clone(),
                            client_secret_env: a.client_secret_env.clone(),
                            scopes: a.scopes.clone(),
                        }),
                },
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
        while let Some(result) = mcp_set.join_next().await {
            match result {
                Ok(Ok((server_name, client, tools))) => {
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
        let caps = orka_os::EnvironmentCapabilities::probe(&config.os).await;
        info!(
            no_new_privileges = caps.no_new_privileges,
            package_updates = caps.package_updates.available,
            systemctl = caps.systemctl.available,
            journalctl = caps.journalctl.available,
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

    let skills = Arc::new(skills);
    info!("skill registry ready ({} skills)", skills.list().len());

    // 4e. Soft skills (SKILL.md-based instruction skills)
    let soft_skills: Option<Arc<orka_skills::SoftSkillRegistry>> =
        if let Some(ref dir) = config.soft_skills.dir {
            let skills_list = orka_skills::scan_soft_skills(std::path::Path::new(dir));
            let mut reg = orka_skills::SoftSkillRegistry::new();
            let count = skills_list.len();
            for skill in skills_list {
                reg.register(skill);
            }
            info!(count, "soft skill registry ready");
            Some(Arc::new(reg))
        } else {
            None
        };

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
                            pc.base_url.clone(),
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
        // Load workspaces in parallel
        let mut load_set = tokio::task::JoinSet::new();
        let entries = config.workspaces.to_vec();
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
                        event_sink_hb.emit(orka_core::DomainEvent::new(
                            orka_core::DomainEventKind::Heartbeat,
                        )).await;
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

    // 6b–6e. Optional adapters (started in parallel).
    // Each adapter block is gated by a feature flag; when disabled, a ready(None) future
    // is used so that tokio::join! can still join all four arms uniformly.

    #[cfg(feature = "telegram")]
    let tg_fut = {
        let secrets = secrets.clone();
        let bus = bus.clone();
        let shutdown = shutdown.clone();
        let tg_config = config.adapters.telegram.clone();
        let tg_memory = memory.clone();
        async move {
            let tg_config = tg_config.as_ref()?;
            let secret_name = tg_config
                .bot_token_secret
                .as_deref()
                .unwrap_or("telegram_bot_token");
            match secrets.get_secret(secret_name).await {
                Ok(secret) => {
                    let token = secret.expose_str().unwrap_or("").to_string();
                    if token.is_empty() {
                        warn!("telegram bot token is empty, adapter disabled");
                        return None;
                    }
                    let tg: Arc<dyn ChannelAdapter> = Arc::new(
                        orka_adapter_telegram::TelegramAdapter::new(tg_config.clone(), token)
                            .with_memory(tg_memory),
                    );
                    if let Err(e) =
                        start_adapter(tg.clone(), bus, shutdown, tg_config.workspace.clone()).await
                    {
                        warn!(%e, "failed to start telegram adapter");
                        return None;
                    }
                    info!("telegram adapter started");
                    Some(tg)
                }
                Err(e) => {
                    warn!(%e, "failed to load telegram bot token, adapter disabled");
                    None
                }
            }
        }
    };
    #[cfg(not(feature = "telegram"))]
    let tg_fut = std::future::ready(None::<Arc<dyn ChannelAdapter>>);

    #[cfg(feature = "discord")]
    let dc_fut = {
        let secrets = secrets.clone();
        let bus = bus.clone();
        let shutdown = shutdown.clone();
        let dc_config = config.adapters.discord.clone();
        async move {
            let dc_config = dc_config.as_ref()?;
            let secret_name = dc_config
                .bot_token_secret
                .as_deref()
                .unwrap_or("discord_bot_token");
            match secrets.get_secret(secret_name).await {
                Ok(secret) => {
                    let token = secret.expose_str().unwrap_or("").to_string();
                    if token.is_empty() {
                        warn!("discord bot token is empty, adapter disabled");
                        return None;
                    }
                    let dc: Arc<dyn ChannelAdapter> =
                        Arc::new(orka_adapter_discord::DiscordAdapter::new(
                            token,
                            dc_config.application_id.clone(),
                        ));
                    if let Err(e) =
                        start_adapter(dc.clone(), bus, shutdown, dc_config.workspace.clone()).await
                    {
                        warn!(%e, "failed to start discord adapter");
                        return None;
                    }
                    info!("discord adapter started");
                    Some(dc)
                }
                Err(e) => {
                    warn!(%e, "failed to load discord bot token, adapter disabled");
                    None
                }
            }
        }
    };
    #[cfg(not(feature = "discord"))]
    let dc_fut = std::future::ready(None::<Arc<dyn ChannelAdapter>>);

    #[cfg(feature = "slack")]
    let slack_fut = {
        let secrets = secrets.clone();
        let bus = bus.clone();
        let shutdown = shutdown.clone();
        let slack_config = config.adapters.slack.clone();
        async move {
            let slack_config = slack_config.as_ref()?;
            let secret_name = slack_config
                .bot_token_secret
                .as_deref()
                .unwrap_or("slack_bot_token");
            match secrets.get_secret(secret_name).await {
                Ok(secret) => {
                    let token = secret.expose_str().unwrap_or("").to_string();
                    if token.is_empty() {
                        warn!("slack bot token is empty, adapter disabled");
                        return None;
                    }
                    let slack: Arc<dyn ChannelAdapter> = Arc::new(
                        orka_adapter_slack::SlackAdapter::new(token, slack_config.listen_port),
                    );
                    if let Err(e) =
                        start_adapter(slack.clone(), bus, shutdown, slack_config.workspace.clone())
                            .await
                    {
                        warn!(%e, "failed to start slack adapter");
                        return None;
                    }
                    info!(port = slack_config.listen_port, "slack adapter started");
                    Some(slack)
                }
                Err(e) => {
                    warn!(%e, "failed to load slack bot token, adapter disabled");
                    None
                }
            }
        }
    };
    #[cfg(not(feature = "slack"))]
    let slack_fut = std::future::ready(None::<Arc<dyn ChannelAdapter>>);

    #[cfg(feature = "whatsapp")]
    let wa_fut = {
        let secrets = secrets.clone();
        let bus = bus.clone();
        let shutdown = shutdown.clone();
        let wa_config = config.adapters.whatsapp.clone();
        async move {
            let wa_config = wa_config.as_ref()?;
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
                    if access_token.is_empty() || phone_id.is_empty() {
                        warn!(
                            "whatsapp access token or phone_number_id is empty, adapter disabled"
                        );
                        return None;
                    }
                    let wa: Arc<dyn ChannelAdapter> =
                        Arc::new(orka_adapter_whatsapp::WhatsAppAdapter::new(
                            access_token,
                            phone_id,
                            verify_token,
                            wa_config.listen_port,
                        ));
                    if let Err(e) =
                        start_adapter(wa.clone(), bus, shutdown, wa_config.workspace.clone()).await
                    {
                        warn!(%e, "failed to start whatsapp adapter");
                        return None;
                    }
                    info!(port = wa_config.listen_port, "whatsapp adapter started");
                    Some(wa)
                }
                _ => {
                    warn!("failed to load whatsapp secrets, adapter disabled");
                    None
                }
            }
        }
    };
    #[cfg(not(feature = "whatsapp"))]
    let wa_fut = std::future::ready(None::<Arc<dyn ChannelAdapter>>);

    let (tg, dc, slack, wa) = tokio::join!(tg_fut, dc_fut, slack_fut, wa_fut);
    adapters.extend(tg);
    adapters.extend(dc);
    adapters.extend(slack);
    adapters.extend(wa);

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
            "/api/v1/version",
            axum::routing::get(|| async {
                axum::Json(serde_json::json!({
                    "version": VERSION,
                    "git_sha": GIT_SHA,
                    "build_date": BUILD_DATE,
                }))
            }),
        )
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
                let qdrant_url = if config.knowledge.enabled {
                    Some(config.knowledge.vector_store.url.clone())
                } else {
                    None
                };
                move || {
                    let queue = queue.clone();
                    let redis_url = redis_url.clone();
                    let qdrant_url = qdrant_url.clone();
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

                        // Qdrant check (only when knowledge is enabled)
                        // Uses gRPC (same protocol as the data path) to avoid
                        // confusing the gRPC port (6334) with the HTTP port (6333).
                        if let Some(ref url) = qdrant_url {
                            match qdrant_client::Qdrant::from_url(url).build() {
                                Ok(client) => {
                                    match tokio::time::timeout(
                                        std::time::Duration::from_secs(2),
                                        client.health_check(),
                                    )
                                    .await
                                    {
                                        Ok(Ok(_)) => {
                                            checks.insert("qdrant".into(), serde_json::json!("ok"));
                                        }
                                        Ok(Err(e)) => {
                                            checks.insert(
                                                "qdrant".into(),
                                                serde_json::json!(format!("error: {e}")),
                                            );
                                            all_ok = false;
                                        }
                                        Err(_) => {
                                            checks.insert(
                                                "qdrant".into(),
                                                serde_json::json!("error: health check timed out"),
                                            );
                                            all_ok = false;
                                        }
                                    }
                                }
                                Err(e) => {
                                    checks.insert(
                                        "qdrant".into(),
                                        serde_json::json!(format!("error: {e}")),
                                    );
                                    all_ok = false;
                                }
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

    // Experience / self-learning service
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

    // 8. Build command registry (for adapter menu hints) and start graph worker pool
    let mut commands = CommandRegistry::new();
    orka_worker::commands::register_all(
        &mut commands,
        skills.clone(),
        memory.clone(),
        secrets.clone(),
        workspace_registry.clone(),
        &config.agent,
        experience_service.clone(),
    );
    let commands = Arc::new(commands);

    // Register commands with adapters (e.g. Telegram command menu)
    {
        let cmd_list = commands.list();
        for adapter in &adapters {
            if let Err(e) = adapter.register_commands(&cmd_list).await {
                warn!(%e, channel = adapter.channel_id(), "failed to register commands with adapter");
            }
        }
    }

    // Build agent graph from config (falls back to single-agent graph for legacy configs)
    let graph = Arc::new(
        orka_agent::build_graph_from_config(&config, &workspace_registry)
            .await
            .context("failed to build agent graph")?,
    );
    info!(graph_id = %graph.id, "agent graph built");

    // 6g. Management API routes — must be built after graph/experience/scheduler are available
    {
        use axum::extract::{Json, Path, Query};
        use axum::http::StatusCode;
        use axum::response::IntoResponse;
        use std::collections::HashMap;

        let scheduler_for_api = scheduler_store.clone();
        let graph_for_api = graph.clone();
        let experience_for_api = experience_service.clone();

        let s1 = skills.clone();
        let s2 = skills.clone();
        let s3 = skills.clone();
        let soft1 = soft_skills.clone();
        let sc1 = scheduler_for_api.clone();
        let sc2 = scheduler_for_api.clone();
        let sc3 = scheduler_for_api.clone();
        let w1 = workspace_registry.clone();
        let w2 = workspace_registry.clone();
        let g1 = graph_for_api.clone();
        let e1 = experience_for_api.clone();
        let e2 = experience_for_api.clone();
        let e3 = experience_for_api.clone();
        let ss1 = sessions.clone();
        let ss2 = sessions.clone();
        let ss3 = sessions.clone();

        let mgmt_routes = axum::Router::new()
            .route("/api/v1/skills", axum::routing::get(move || {
                let skills = s1.clone();
                async move {
                    let list: Vec<serde_json::Value> = skills.list_info().iter().map(|(name, skill, state)| {
                        let status = match state {
                            orka_circuit_breaker::CircuitState::Closed => "ok",
                            orka_circuit_breaker::CircuitState::HalfOpen => "degraded",
                            orka_circuit_breaker::CircuitState::Open => "disabled",
                            _ => "ok",
                        };
                        serde_json::json!({
                            "name": name,
                            "category": skill.category(),
                            "description": skill.description(),
                            "status": status,
                            "schema": skill.schema(),
                        })
                    }).collect();
                    axum::Json(list)
                }
            }))
            .route("/api/v1/soft-skills", axum::routing::get(move || {
                let reg = soft1.clone();
                async move {
                    let list: Vec<serde_json::Value> = reg
                        .as_deref()
                        .map(|r| r.summaries())
                        .unwrap_or_default()
                        .into_iter()
                        .map(|s| serde_json::json!({
                            "name": s.name,
                            "description": s.description,
                            "tags": s.tags,
                        }))
                        .collect();
                    axum::Json(list)
                }
            }))
            .route("/api/v1/skills/{name}", axum::routing::get(move |Path(name): Path<String>| {
                let skills = s2.clone();
                async move {
                    match skills.get(&name) {
                        Some(skill) => axum::Json(serde_json::json!({
                            "name": skill.name(),
                            "description": skill.description(),
                            "schema": skill.schema(),
                        })).into_response(),
                        None => (StatusCode::NOT_FOUND, format!("skill '{name}' not found")).into_response(),
                    }
                }
            }))
            .route("/api/v1/eval", axum::routing::post(move |axum::Json(body): axum::Json<serde_json::Value>| {
                let skills = s3.clone();
                async move {
                    let skill_filter = body["skill"].as_str().map(String::from);
                    let dir = body["dir"].as_str().unwrap_or("evals").to_string();
                    let runner = orka_eval::EvalRunner::new(skills);
                    match runner.run_dir(std::path::Path::new(&dir), skill_filter.as_deref()).await {
                        Ok(report) => {
                            let json_str = report.to_json();
                            let val: serde_json::Value = serde_json::from_str(&json_str).unwrap_or_default();
                            axum::Json(val).into_response()
                        }
                        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("eval failed: {e}")).into_response(),
                    }
                }
            }))
            .route("/api/v1/schedules",
                axum::routing::get(move || {
                    let store = sc1.clone();
                    async move {
                        let Some(store) = store else {
                            return (StatusCode::SERVICE_UNAVAILABLE, "scheduler not enabled").into_response();
                        };
                        match store.list(false).await {
                            Ok(s) => axum::Json(s).into_response(),
                            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("list failed: {e}")).into_response(),
                        }
                    }
                })
                .post(move |Json(body): Json<serde_json::Value>| {
                    let store = sc2.clone();
                    async move {
                        let Some(store) = store else {
                            return (StatusCode::SERVICE_UNAVAILABLE, "scheduler not enabled").into_response();
                        };
                        let name = match body["name"].as_str() {
                            Some(n) => n.to_string(),
                            None => return (StatusCode::BAD_REQUEST, "'name' is required").into_response(),
                        };
                        let cron_expr = body["cron"].as_str().map(String::from);
                        let run_at_str = body["run_at"].as_str().map(String::from);
                        if cron_expr.is_none() && run_at_str.is_none() {
                            return (StatusCode::BAD_REQUEST, "either 'cron' or 'run_at' is required").into_response();
                        }
                        let next_run = if let Some(ref cron_str) = cron_expr {
                            use std::str::FromStr as _;
                            match cron::Schedule::from_str(cron_str) {
                                Ok(sched) => match sched.upcoming(chrono::Utc).next().map(|t| t.timestamp()) {
                                    Some(ts) => ts,
                                    None => return (StatusCode::BAD_REQUEST, "no upcoming run for cron").into_response(),
                                },
                                Err(e) => return (StatusCode::BAD_REQUEST, format!("invalid cron: {e}")).into_response(),
                            }
                        } else if let Some(ref run_at) = run_at_str {
                            match chrono::DateTime::parse_from_rfc3339(run_at) {
                                Ok(dt) => dt.timestamp(),
                                Err(e) => return (StatusCode::BAD_REQUEST, format!("invalid run_at: {e}")).into_response(),
                            }
                        } else { 0 };
                        let args: Option<std::collections::HashMap<String, serde_json::Value>> = body["args"]
                            .as_object()
                            .map(|m| m.iter().map(|(k, v)| (k.clone(), v.clone())).collect());
                        let schedule = orka_scheduler::types::Schedule {
                            id: uuid::Uuid::now_v7().to_string(),
                            name,
                            cron: cron_expr,
                            run_at: run_at_str,
                            timezone: body["timezone"].as_str().map(String::from),
                            skill: body["skill"].as_str().map(String::from),
                            args,
                            message: body["message"].as_str().map(String::from),
                            next_run,
                            created_at: chrono::Utc::now().to_rfc3339(),
                            completed: false,
                        };
                        match store.add(&schedule).await {
                            Ok(()) => (StatusCode::CREATED, axum::Json(schedule)).into_response(),
                            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("create failed: {e}")).into_response(),
                        }
                    }
                })
            )
            .route("/api/v1/schedules/{id}", axum::routing::delete(move |Path(id): Path<String>| {
                let store = sc3.clone();
                async move {
                    let Some(store) = store else {
                        return (StatusCode::SERVICE_UNAVAILABLE, "scheduler not enabled").into_response();
                    };
                    match store.remove(&id).await {
                        Ok(found) => axum::Json(serde_json::json!({ "deleted": found })).into_response(),
                        Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("delete failed: {e}")).into_response(),
                    }
                }
            }))
            .route("/api/v1/workspaces", axum::routing::get(move || {
                let registry = w1.clone();
                async move {
                    let mut list = Vec::new();
                    for name in registry.list_names() {
                        if let Some(loader) = registry.get(name) {
                            let state = loader.state();
                            let state = state.read().await;
                            let (agent_name, description) = state.soul.as_ref()
                                .map(|d| (d.frontmatter.name.clone(), d.frontmatter.description.clone()))
                                .unwrap_or((None, None));
                            list.push(serde_json::json!({
                                "name": name,
                                "agent_name": agent_name,
                                "description": description,
                                "has_tools": state.tools_body.is_some(),
                            }));
                        }
                    }
                    axum::Json(list)
                }
            }))
            .route("/api/v1/workspaces/{name}", axum::routing::get(move |Path(ws_name): Path<String>| {
                let registry = w2.clone();
                async move {
                    match registry.get(&ws_name) {
                        None => (StatusCode::NOT_FOUND, format!("workspace '{ws_name}' not found")).into_response(),
                        Some(loader) => {
                            let state = loader.state();
                            let state = state.read().await;
                            let fm = state.soul.as_ref().map(|d| &d.frontmatter);
                            axum::Json(serde_json::json!({
                                "name": ws_name,
                                "agent_name": fm.and_then(|f| f.name.as_deref()),
                                "description": fm.and_then(|f| f.description.as_deref()),
                                "version": fm.and_then(|f| f.version.as_deref()),
                                "soul_body": state.soul.as_ref().map(|d| d.body.as_str()),
                                "tools_body": state.tools_body.as_deref(),
                            })).into_response()
                        }
                    }
                }
            }))
            .route("/api/v1/graph", axum::routing::get(move || {
                let g = g1.clone();
                async move {
                    let nodes: Vec<serde_json::Value> = g.nodes_iter().map(|(id, node)| {
                        serde_json::json!({
                            "id": id.to_string(),
                            "kind": format!("{:?}", node.kind),
                            "agent": {
                                "id": node.agent.id.to_string(),
                                "name": node.agent.display_name,
                                "max_iterations": node.agent.max_iterations,
                                "handoff_targets": node.agent.handoff_targets.iter()
                                    .map(|t| t.to_string()).collect::<Vec<_>>(),
                            }
                        })
                    }).collect();
                    let edges: Vec<serde_json::Value> = g.edges_iter().flat_map(|(from, edges)| {
                        let from = from.to_string();
                        edges.iter().map(move |e| {
                            let condition = match &e.condition {
                                None => serde_json::json!("always"),
                                Some(orka_agent::EdgeCondition::Always) => serde_json::json!("always"),
                                Some(orka_agent::EdgeCondition::OutputContains(s)) => {
                                    serde_json::json!({"output_contains": s})
                                }
                                Some(orka_agent::EdgeCondition::StateMatch { key, pattern }) => {
                                    serde_json::json!({
                                        "state_match": {
                                            "key": format!("{}.{}", key.namespace, key.name),
                                            "pattern": pattern,
                                        }
                                    })
                                }
                            };
                            serde_json::json!({
                                "from": from,
                                "to": e.target.to_string(),
                                "priority": e.priority,
                                "condition": condition,
                            })
                        }).collect::<Vec<_>>()
                    }).collect();
                    axum::Json(serde_json::json!({
                        "id": g.id,
                        "entry": g.entry.to_string(),
                        "nodes": nodes,
                        "edges": edges,
                        "termination": {
                            "max_total_iterations": g.termination.max_total_iterations,
                            "max_total_tokens": g.termination.max_total_tokens,
                            "max_duration_secs": g.termination.max_duration.as_secs(),
                            "terminal_agents": g.termination.terminal_agents.iter()
                                .map(|a| a.to_string()).collect::<Vec<_>>(),
                        }
                    }))
                }
            }))
            .route("/api/v1/experience/status", axum::routing::get(move || {
                let exp = e1.clone();
                async move {
                    axum::Json(serde_json::json!({
                        "enabled": exp.as_ref().map(|e| e.is_enabled()).unwrap_or(false),
                    }))
                }
            }))
            .route("/api/v1/experience/principles", axum::routing::get(
                move |Query(params): Query<HashMap<String, String>>| {
                    let exp = e2.clone();
                    async move {
                        let Some(exp) = exp else {
                            return (StatusCode::SERVICE_UNAVAILABLE, "experience not enabled").into_response();
                        };
                        let workspace = params.get("workspace").map(String::as_str).unwrap_or("default");
                        let query = params.get("query").map(String::as_str).unwrap_or("");
                        let limit: usize = params.get("limit").and_then(|l| l.parse().ok()).unwrap_or(10);
                        match exp.retrieve_principles(query, workspace).await {
                            Ok(mut principles) => {
                                principles.truncate(limit);
                                axum::Json(principles).into_response()
                            }
                            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("retrieve failed: {e}")).into_response(),
                        }
                    }
                }
            ))
            .route("/api/v1/experience/distill", axum::routing::post(
                move |Json(body): Json<serde_json::Value>| {
                    let exp = e3.clone();
                    async move {
                        let Some(exp) = exp else {
                            return (StatusCode::SERVICE_UNAVAILABLE, "experience not enabled").into_response();
                        };
                        let workspace = body["workspace"].as_str().unwrap_or("default");
                        match exp.distill(workspace).await {
                            Ok(created) => axum::Json(serde_json::json!({ "created": created })).into_response(),
                            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("distill failed: {e}")).into_response(),
                        }
                    }
                }
            ))
            .route("/api/v1/sessions",
                axum::routing::get(move |Query(params): Query<HashMap<String, String>>| {
                    let sessions = ss1.clone();
                    async move {
                        let limit: usize = params.get("limit").and_then(|l| l.parse().ok()).unwrap_or(20);
                        match sessions.list(limit).await {
                            Ok(list) => axum::Json(list).into_response(),
                            Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("list failed: {e}")).into_response(),
                        }
                    }
                })
            )
            .route("/api/v1/sessions/{id}",
                axum::routing::get(move |Path(id): Path<String>| {
                    let sessions = ss2.clone();
                    async move {
                        match uuid::Uuid::parse_str(&id) {
                            Err(_) => (StatusCode::BAD_REQUEST, "invalid session ID").into_response(),
                            Ok(uuid) => {
                                let sid = orka_core::SessionId(uuid);
                                match sessions.get(&sid).await {
                                    Ok(Some(s)) => axum::Json(s).into_response(),
                                    Ok(None) => (StatusCode::NOT_FOUND, "session not found").into_response(),
                                    Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("get failed: {e}")).into_response(),
                                }
                            }
                        }
                    }
                })
                .delete(move |Path(id): Path<String>| {
                    let sessions = ss3.clone();
                    async move {
                        match uuid::Uuid::parse_str(&id) {
                            Err(_) => (StatusCode::BAD_REQUEST, "invalid session ID").into_response(),
                            Ok(uuid) => {
                                let sid = orka_core::SessionId(uuid);
                                match sessions.delete(&sid).await {
                                    Ok(()) => axum::Json(serde_json::json!({ "deleted": true })).into_response(),
                                    Err(e) => (StatusCode::INTERNAL_SERVER_ERROR, format!("delete failed: {e}")).into_response(),
                                }
                            }
                        }
                    }
                })
            );

        let api_routes = api_routes.merge(mgmt_routes);

        // Apply auth to all protected API routes
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
            let base_url =
                config.a2a.url.clone().unwrap_or_else(|| {
                    format!("http://{}:{}", config.server.host, config.server.port)
                });
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
            .merge(SwaggerUi::new("/docs").url("/api-doc/openapi.json", {
                let mut doc = ApiDoc::openapi();
                doc.info.version = VERSION.to_string();
                doc
            }))
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
    }

    let memory_for_worker = memory.clone();
    let executor = Arc::new(orka_agent::GraphExecutor::new(orka_agent::ExecutorDeps {
        skills: skills.clone(),
        memory,
        secrets,
        llm: llm_client,
        event_sink: event_sink.clone(),
        stream_registry: stream_registry.clone(),
        experience: experience_service.clone(),
        soft_skills: soft_skills.clone(),
    }));

    let worker_pool = WorkerPoolGraph::new(
        queue.clone(),
        sessions.clone(),
        bus.clone(),
        executor,
        graph,
        event_sink.clone(),
        config.worker.concurrency,
        config.queue.max_retries,
    )
    .with_retry_delay(config.worker.retry_base_delay_ms)
    .with_memory(memory_for_worker);
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

    // 8c. Distillation loop (if experience enabled and interval > 0)
    let _distillation_handle = if let Some(ref exp) = experience_service {
        let interval_secs = config.experience.distillation_interval_secs;
        if interval_secs > 0 {
            let exp = exp.clone();
            let workspace_names: Vec<String> = workspace_registry
                .list_names()
                .into_iter()
                .map(|s| s.to_string())
                .collect();
            let distill_event_sink = event_sink.clone();
            let distill_cancel = shutdown.clone();
            Some(tokio::spawn(async move {
                let mut interval =
                    tokio::time::interval(std::time::Duration::from_secs(interval_secs));
                // Skip the immediate first tick so distillation runs after some data is collected
                interval.tick().await;
                loop {
                    tokio::select! {
                        _ = distill_cancel.cancelled() => break,
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
                            let mut outbound = OutboundMessage::text(
                                envelope.channel.clone(),
                                envelope.session_id,
                                text,
                                None,
                            );
                            outbound.metadata = envelope.metadata.clone();
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_gt_newer_major() {
        assert!(semver_gt("2.0.0", "1.0.0"));
    }

    #[test]
    fn semver_gt_older_returns_false() {
        assert!(!semver_gt("1.0.0", "2.0.0"));
    }

    #[test]
    fn semver_gt_equal_returns_false() {
        assert!(!semver_gt("1.2.3", "1.2.3"));
    }

    #[test]
    fn default_env_var_known_and_unknown() {
        assert_eq!(default_env_var("anthropic"), "ANTHROPIC_API_KEY");
        assert_eq!(default_env_var("openai"), "OPENAI_API_KEY");
        assert_eq!(default_env_var("unknown_provider"), "");
    }
}
