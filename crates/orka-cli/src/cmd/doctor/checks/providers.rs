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
            id: CheckId("PRV-001"),
            category: Category::Providers,
            severity: Severity::Critical,
            name: "At least one LLM provider",
            description: "At least one entry in llm.providers must be configured.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        let config = match &ctx.config {
            Some(c) => c,
            None => return CheckOutcome::skip("config not loaded"),
        };

        if config.llm.providers.is_empty() {
            CheckOutcome::fail("no LLM providers configured")
                .with_hint(
                    "Add at least one [[llm.providers]] entry to orka.toml. \
                     Example: provider = \"anthropic\", api_key_env = \"ANTHROPIC_API_KEY\".",
                )
        } else {
            CheckOutcome::pass(format!("{} provider(s)", config.llm.providers.len()))
                .with_detail(
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
            id: CheckId("PRV-002"),
            category: Category::Providers,
            severity: Severity::Error,
            name: "API keys resolvable",
            description: "Each LLM provider must have a resolvable API key source.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        let config = match &ctx.config {
            Some(c) => c,
            None => return CheckOutcome::skip("config not loaded"),
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
                 default environment variable (ANTHROPIC_API_KEY, OPENAI_API_KEY, etc.).",
            )
            .with_detail(sources.join(", "))
        }
    }

    fn explain(&self) -> &'static str {
        "Orka resolves API keys in this order: \
         1) inline api_key in config (not recommended), \
         2) api_key_env — an environment variable name specified in config, \
         3) the provider's default environment variable (ANTHROPIC_API_KEY, OPENAI_API_KEY), \
         4) api_key_secret — a path in the Redis secret store. \
         This check verifies that at least one source resolves for each provider \
         without revealing the actual key value."
    }
}

/// Resolve where a provider's API key comes from. Returns None if no source is configured.
fn resolve_api_key(provider: &orka_core::config::LlmProviderConfig) -> Option<String> {
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

    // 4. Secret store path configured (we can't resolve it without Redis connection)
    if provider.api_key_secret.as_deref().is_some_and(|s| !s.is_empty()) {
        return Some("secret store (requires Redis)".to_string());
    }

    None
}

#[async_trait]
impl DoctorCheck for PrvProviderReachable {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId("PRV-003"),
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

        let config = match &ctx.config {
            Some(c) => c,
            None => return CheckOutcome::skip("config not loaded"),
        };

        if config.llm.providers.is_empty() {
            return CheckOutcome::skip("no providers configured");
        }

        let mut results = Vec::new();

        for provider in &config.llm.providers {
            let reachable = probe_provider(provider).await;
            results.push(format!("{}: {}", provider.name, reachable));
        }

        CheckOutcome::pass("probe complete").with_detail(results.join(", "))
    }
}

async fn probe_provider(provider: &orka_core::config::LlmProviderConfig) -> &'static str {
    let base_url = match provider.provider.as_str() {
        "anthropic" => provider
            .base_url
            .as_deref()
            .unwrap_or("https://api.anthropic.com"),
        "openai" => provider
            .base_url
            .as_deref()
            .unwrap_or("https://api.openai.com"),
        "ollama" => provider
            .base_url
            .as_deref()
            .unwrap_or("http://localhost:11434"),
        _ => {
            if let Some(url) = &provider.base_url {
                url.as_str()
            } else {
                return "unknown provider";
            }
        }
    };

    // We just check TCP connectivity to the host, not a full API call
    let url = match base_url.parse::<reqwest::Url>() {
        Ok(u) => u,
        Err(_) => return "invalid URL",
    };

    let host = match url.host_str() {
        Some(h) => h.to_string(),
        None => return "no host",
    };
    let port = url.port_or_known_default().unwrap_or(443);

    let addr = format!("{host}:{port}");
    match tokio::time::timeout(
        std::time::Duration::from_secs(3),
        tokio::net::TcpStream::connect(&addr),
    )
    .await
    {
        Ok(Ok(_)) => "reachable",
        Ok(Err(_)) => "unreachable",
        Err(_) => "timeout",
    }
}

#[async_trait]
impl DoctorCheck for PrvEmbeddingProvider {
    fn meta(&self) -> CheckMeta {
        CheckMeta {
            id: CheckId("PRV-004"),
            category: Category::Providers,
            severity: Severity::Error,
            name: "Embedding provider configured",
            description: "An embedding provider must be configured when knowledge.enabled = true.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        let config = match &ctx.config {
            Some(c) => c,
            None => return CheckOutcome::skip("config not loaded"),
        };

        if !config.knowledge.enabled {
            return CheckOutcome::skip("knowledge.enabled = false");
        }

        use orka_core::config::EmbeddingProvider;
        let provider = &config.knowledge.embeddings.provider;
        if *provider == EmbeddingProvider::Local {
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
            id: CheckId("PRV-005"),
            category: Category::Providers,
            severity: Severity::Warning,
            name: "Web search API key",
            description: "A web search API key must be resolvable when web search is configured.",
        }
    }

    async fn run(&self, ctx: &CheckContext) -> CheckOutcome {
        let config = match &ctx.config {
            Some(c) => c,
            None => return CheckOutcome::skip("config not loaded"),
        };

        let provider = &config.web.search_provider;
        if provider == "none" || provider.is_empty() {
            return CheckOutcome::skip("web search provider = none");
        }

        match provider.as_str() {
            "searxng" => {
                let url = config.web.searxng_base_url.as_deref().unwrap_or("");
                if url.is_empty() {
                    CheckOutcome::fail("searxng_base_url not configured")
                        .with_hint("Set web.searxng_base_url to your SearXNG instance URL.")
                } else {
                    CheckOutcome::pass(format!("searxng at {url}"))
                }
            }
            "tavily" => check_web_api_key(config, "TAVILY_API_KEY", provider),
            "brave" => check_web_api_key(config, "BRAVE_API_KEY", provider),
            other => CheckOutcome::skip(format!("unknown provider: {other}")),
        }
    }
}

fn check_web_api_key(
    config: &orka_core::config::OrkaConfig,
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

    CheckOutcome::fail(format!("{provider}: no API key found"))
        .with_hint(format!(
            "Set web.api_key_env = \"{default_env}\" and export the variable, \
             or set web.search_provider = \"none\" to disable web search."
        ))
}
