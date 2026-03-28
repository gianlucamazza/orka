use async_trait::async_trait;

use crate::cmd::doctor::{
    CheckContext, DoctorCheck,
    types::{Category, CheckId, CheckMeta, CheckOutcome, Severity},
};

pub struct PrvAtLeastOneProvider;
pub struct PrvApiKeysResolvable;
pub struct PrvProviderReachable;
pub struct PrvEmbeddingProvider;
pub struct PrvWebSearchKey;

#[async_trait]
impl DoctorCheck for PrvAtLeastOneProvider {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId::new("PRV-001"),
            category: Category::Providers,
            severity: Severity::Critical,
            name: "At least one LLM provider",
            description: "At least one entry in llm.providers must be configured.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        let Some(config) = &ctx.config else {
            return CheckOutcome::skip("config not loaded");
        };

        if config.llm.providers.is_empty() {
            CheckOutcome::fail("no LLM providers configured").with_hint(
                "Add at least one [[llm.providers]] entry to orka.toml. \
                     Example: provider = \"anthropic\", api_key_env = \"ANTHROPIC_API_KEY\".",
            )
        } else {
            CheckOutcome::pass(format!("{} provider(s)", config.llm.providers.len())).with_detail(
                config
                    .llm
                    .providers
                    .iter()
                    .map(|p| format!("{} ({})", p.name, p.provider))
                    .collect::<Vec<_>>()
                    .join(", "),
            )
        }
    }
}

#[async_trait]
impl DoctorCheck for PrvApiKeysResolvable {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId::new("PRV-002"),
            category: Category::Providers,
            severity: Severity::Error,
            name: "API keys resolvable",
            description: "Each LLM provider must have a resolvable API key source.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        let Some(config) = &ctx.config else {
            return CheckOutcome::skip("config not loaded");
        };

        if config.llm.providers.is_empty() {
            return CheckOutcome::skip("no providers configured");
        }

        let mut missing = Vec::new();
        let mut sources = Vec::new();

        for provider in &config.llm.providers {
            // Ollama doesn't need an API key
            if provider.provider == "ollama" {
                sources.push(format!("{}: no key required (ollama)", provider.name));
                continue;
            }

            let resolved = resolve_api_key(provider);
            match resolved {
                Some(source) => sources.push(format!("{}: {source}", provider.name)),
                None => missing.push(provider.name.clone()),
            }
        }

        if missing.is_empty() {
            CheckOutcome::pass(format!("{} provider(s) have API keys", sources.len()))
                .with_detail(sources.join(", "))
        } else {
            CheckOutcome::fail(format!(
                "{} provider(s) missing API key: {}",
                missing.len(),
                missing.join(", ")
            ))
            .with_hint(
                "Set api_key_env = \"YOUR_ENV_VAR\" in the provider config, or set the \
                 default environment variable (ANTHROPIC_API_KEY, MOONSHOT_API_KEY, OPENAI_API_KEY, etc.).",
            )
            .with_detail(sources.join(", "))
        }
    }

    fn explain(&self) -> &'static str {
        "Orka resolves API keys in this order: \
         1) explicit auth_token/auth_token_env/default auth-token env when auth_kind requires bearer auth, \
         2) inline api_key in config (not recommended), \
         3) api_key_env — an environment variable name specified in config, \
         4) the provider's default environment variable (ANTHROPIC_API_KEY, MOONSHOT_API_KEY, OPENAI_API_KEY), \
         5) api_key_secret or auth_token_secret — a path in the secret store. \
         This check verifies that at least one source resolves for each provider \
         without revealing the actual key value."
    }
}

/// Resolve where a provider credential comes from. Returns None if no source is
/// configured.
fn resolve_api_key(provider: &orka_llm::LlmProviderConfig) -> Option<String> {
    use orka_llm::LlmAuthKind;

    let auth_kind = provider.auth_kind;

    let resolve_auth_token = || {
        // 1. Inline auth token
        if provider
            .auth_token
            .as_deref()
            .is_some_and(|k| !k.is_empty())
        {
            return Some("inline (auth_token)".to_string());
        }

        // 2. Explicit auth-token env var
        if let Some(env_name) = &provider.auth_token_env
            && std::env::var(env_name).is_ok()
        {
            return Some(format!("env:{env_name} (auth_token)"));
        }

        // 3. Default auth-token env var by provider type
        let default_env = match provider.provider.as_str() {
            "anthropic" => Some("ANTHROPIC_AUTH_TOKEN"),
            _ => None,
        };
        if let Some(env_name) = default_env
            && std::env::var(env_name).is_ok()
        {
            return Some(format!("env:{env_name} (default auth_token)"));
        }

        // 4. Secret store path configured
        if provider
            .auth_token_secret
            .as_deref()
            .is_some_and(|s| !s.is_empty())
        {
            return Some("secret store (auth_token)".to_string());
        }

        None
    };

    let resolve_api_key = || {
        // 1. Inline key
        if provider.api_key.as_deref().is_some_and(|k| !k.is_empty()) {
            return Some("inline (api_key)".to_string());
        }

        // 2. Explicit env var
        if let Some(env_name) = &provider.api_key_env
            && std::env::var(env_name).is_ok()
        {
            return Some(format!("env:{env_name}"));
        }

        // 3. Default env var by provider type
        let default_env = match provider.provider.as_str() {
            "anthropic" => Some("ANTHROPIC_API_KEY"),
            "openai" => Some("OPENAI_API_KEY"),
            "moonshot" => Some("MOONSHOT_API_KEY"),
            "groq" => Some("GROQ_API_KEY"),
            "mistral" => Some("MISTRAL_API_KEY"),
            "together" => Some("TOGETHER_API_KEY"),
            _ => None,
        };
        if let Some(env_name) = default_env
            && std::env::var(env_name).is_ok()
        {
            return Some(format!("env:{env_name} (default)"));
        }

        // 4. Secret store path configured (we can't resolve it without Redis
        //    connection)
        if provider
            .api_key_secret
            .as_deref()
            .is_some_and(|s| !s.is_empty())
        {
            return Some("secret store (requires Redis)".to_string());
        }

        None
    };

    match auth_kind {
        LlmAuthKind::ApiKey => resolve_api_key(),
        LlmAuthKind::AuthToken | LlmAuthKind::Subscription => {
            resolve_auth_token().or_else(resolve_api_key)
        }
        LlmAuthKind::Cli => Some("cli backend (no HTTP credential)".to_string()),
        _ => resolve_auth_token().or_else(resolve_api_key),
    }
}

#[async_trait]
impl DoctorCheck for PrvProviderReachable {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId::new("PRV-003"),
            category: Category::Providers,
            severity: Severity::Info,
            name: "Provider reachable",
            description: "Performs a lightweight connectivity probe to each LLM provider API.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        // Only run this check in verbose mode to avoid network overhead by default
        if !ctx.verbose {
            return CheckOutcome::skip("run with --verbose to probe provider endpoints");
        }

        let Some(config) = &ctx.config else {
            return CheckOutcome::skip("config not loaded");
        };

        if config.llm.providers.is_empty() {
            return CheckOutcome::skip("no providers configured");
        }

        let timeout = ctx.timeout;
        let mut join_set = tokio::task::JoinSet::new();

        for provider in &config.llm.providers {
            let name = provider.name.clone();
            let provider_type = provider.provider.clone();
            let base_url = provider.base_url.clone();
            join_set.spawn(async move {
                let status = probe_provider_url(&provider_type, base_url.as_deref(), timeout).await;
                (name, status)
            });
        }

        let mut results = Vec::new();
        let mut failures = Vec::new();

        while let Some(res) = join_set.join_next().await {
            if let Ok((name, status)) = res {
                results.push(format!("{name}: {status}"));
                if status != "reachable" {
                    failures.push(format!("{name} ({status})"));
                }
            }
        }

        // Sort for deterministic output
        results.sort();
        failures.sort();

        if failures.is_empty() {
            CheckOutcome::pass(format!("{} provider(s) reachable", results.len()))
                .with_detail(results.join(", "))
        } else {
            CheckOutcome::fail(format!(
                "{} provider(s) unreachable: {}",
                failures.len(),
                failures.join(", ")
            ))
            .with_detail(results.join(", "))
            .with_hint("Check network connectivity or provider base_url configuration.")
        }
    }
}

async fn probe_provider_url(
    provider_type: &str,
    base_url: Option<&str>,
    timeout: std::time::Duration,
) -> &'static str {
    let resolved_url = match provider_type {
        "anthropic" => base_url.unwrap_or("https://api.anthropic.com"),
        "openai" => base_url.unwrap_or("https://api.openai.com"),
        "moonshot" => base_url.unwrap_or("https://api.moonshot.ai"),
        "ollama" => base_url.unwrap_or("http://localhost:11434"),
        _ => match base_url {
            Some(url) => url,
            None => return "unknown provider",
        },
    };

    // We just check TCP connectivity to the host, not a full API call
    let Ok(url) = resolved_url.parse::<reqwest::Url>() else {
        return "invalid URL";
    };

    let Some(host) = url.host_str().map(str::to_string) else {
        return "no host";
    };
    let port = url.port_or_known_default().unwrap_or(443);

    let addr = format!("{host}:{port}");
    match tokio::time::timeout(timeout, tokio::net::TcpStream::connect(&addr)).await {
        Ok(Ok(_)) => "reachable",
        Ok(Err(_)) => "unreachable",
        Err(_) => "timeout",
    }
}

#[async_trait]
impl DoctorCheck for PrvEmbeddingProvider {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId::new("PRV-004"),
            category: Category::Providers,
            severity: Severity::Error,
            name: "Embedding provider configured",
            description: "An embedding provider must be configured when knowledge.enabled = true.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        use orka_config::EmbeddingProviderKind;
        let Some(config) = &ctx.config else {
            return CheckOutcome::skip("config not loaded");
        };

        if !config.knowledge.enabled {
            return CheckOutcome::skip("knowledge.enabled = false");
        }

        let provider = &config.knowledge.embeddings.provider;
        if *provider == EmbeddingProviderKind::Local {
            // Local (fastembed) doesn't need an API key
            CheckOutcome::pass("provider: local (fastembed, no API key needed)")
        } else {
            CheckOutcome::pass(format!("provider: {provider:?}"))
        }
    }
}

#[async_trait]
impl DoctorCheck for PrvWebSearchKey {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId::new("PRV-005"),
            category: Category::Providers,
            severity: Severity::Warning,
            name: "Web search API key",
            description: "A web search API key must be resolvable when web search is configured.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        use orka_config::SearchProviderKind;

        let Some(config) = &ctx.config else {
            return CheckOutcome::skip("config not loaded");
        };

        let provider = &config.web.search_provider;
        if *provider == SearchProviderKind::None {
            return CheckOutcome::skip("web search provider = none");
        }

        match provider {
            SearchProviderKind::Searxng => {
                let url = config.web.searxng_base_url.as_deref().unwrap_or("");
                if url.is_empty() {
                    CheckOutcome::fail("searxng_base_url not configured")
                        .with_hint("Set web.searxng_base_url to your SearXNG instance URL.")
                } else {
                    CheckOutcome::pass(format!("searxng at {url}"))
                }
            }
            SearchProviderKind::Tavily => check_web_api_key(config, "TAVILY_API_KEY", "tavily"),
            SearchProviderKind::Brave => check_web_api_key(config, "BRAVE_API_KEY", "brave"),
            SearchProviderKind::None => CheckOutcome::skip("web search provider = none"),
        }
    }
}

fn check_web_api_key(
    config: &orka_config::OrkaConfig,
    default_env: &str,
    provider: &str,
) -> CheckOutcome {
    // Inline key
    if config.web.api_key.as_deref().is_some_and(|k| !k.is_empty()) {
        return CheckOutcome::pass(format!("{provider}: inline key (prefer api_key_env)"));
    }

    // Explicit env var
    if let Some(env_name) = &config.web.api_key_env
        && std::env::var(env_name).is_ok()
    {
        return CheckOutcome::pass(format!("{provider}: env:{env_name}"));
    }

    // Default env var
    if std::env::var(default_env).is_ok() {
        return CheckOutcome::pass(format!("{provider}: env:{default_env} (default)"));
    }

    CheckOutcome::fail(format!("{provider}: no API key found")).with_hint(format!(
        "Set web.api_key_env = \"{default_env}\" and export the variable, \
             or set web.search_provider = \"none\" to disable web search."
    ))
}
