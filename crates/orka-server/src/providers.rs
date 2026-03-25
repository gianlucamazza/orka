//! LLM provider resolution: API key lookup and client construction.

use std::{collections::HashMap, sync::Arc};

use orka_core::{
    config::{LlmProviderConfig, OrkaConfig},
    traits::SecretManager,
};
use orka_llm::SwappableLlmClient;
use tracing::{info, warn};

/// Default environment variable name for a provider's API key.
pub(crate) fn default_env_var(provider: &str) -> &str {
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

/// Result of building all LLM clients from config.
pub(crate) struct LlmClients {
    /// The composed LLM client (router or single), ready to use.
    pub client: Option<Arc<dyn orka_llm::LlmClient>>,
    /// Swappable wrappers keyed by provider name, used for hot-reload.
    pub swappable: HashMap<String, Arc<SwappableLlmClient>>,
}

/// Build the LLM client(s) from the config, resolving API keys from the secret
/// store.
pub(crate) async fn build_llm_clients(
    config: &OrkaConfig,
    secrets: &dyn SecretManager,
) -> LlmClients {
    let mut swappable_clients: HashMap<String, Arc<SwappableLlmClient>> = HashMap::new();

    if config.llm.providers.is_empty() {
        return LlmClients {
            client: None,
            swappable: swappable_clients,
        };
    }

    let mut clients: Vec<(String, Arc<dyn orka_llm::LlmClient>, Vec<String>)> = Vec::new();

    for pc in &config.llm.providers {
        let client: Option<Arc<dyn orka_llm::LlmClient>> = match pc.provider.as_str() {
            "anthropic" => {
                let key = resolve_api_key("anthropic", pc, secrets).await;
                key.map(|k| {
                    let model = pc
                        .model
                        .clone()
                        .unwrap_or_else(|| config.llm.default_model.clone());
                    Arc::new(orka_llm::AnthropicClient::with_options(
                        k,
                        model,
                        pc.timeout_secs
                            .unwrap_or(orka_core::config::defaults::default_llm_timeout_secs()),
                        pc.max_tokens
                            .unwrap_or(orka_core::config::defaults::default_llm_max_tokens()),
                        pc.max_retries
                            .unwrap_or(orka_core::config::defaults::default_llm_max_retries()),
                        orka_llm::ANTHROPIC_API_VERSION.into(),
                        pc.base_url.clone(),
                    )) as Arc<dyn orka_llm::LlmClient>
                })
            }
            "openai" => {
                let key = resolve_api_key("openai", pc, secrets).await;
                key.map(|k| {
                    let model = pc
                        .model
                        .clone()
                        .unwrap_or_else(|| config.llm.default_model.clone());
                    let url = pc
                        .base_url
                        .clone()
                        .unwrap_or_else(|| "https://api.openai.com/v1".into());
                    Arc::new(orka_llm::OpenAiClient::with_options(
                        k,
                        model,
                        pc.timeout_secs
                            .unwrap_or(orka_core::config::defaults::default_llm_timeout_secs()),
                        pc.max_tokens
                            .unwrap_or(orka_core::config::defaults::default_llm_max_tokens()),
                        pc.max_retries
                            .unwrap_or(orka_core::config::defaults::default_llm_max_retries()),
                        url,
                    )) as Arc<dyn orka_llm::LlmClient>
                })
            }
            "ollama" => {
                let model = pc
                    .model
                    .clone()
                    .unwrap_or_else(|| config.llm.default_model.clone());
                let url = pc
                    .base_url
                    .clone()
                    .unwrap_or_else(|| "http://localhost:11434/v1".into());
                Some(Arc::new(orka_llm::OllamaClient::with_options(
                    model,
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
            info!(provider = %pc.name, model = ?pc.model, "LLM provider initialized");
            let swappable = Arc::new(SwappableLlmClient::new(c));
            swappable_clients.insert(pc.name.clone(), swappable.clone());
            clients.push((
                pc.name.clone(),
                swappable as Arc<dyn orka_llm::LlmClient>,
                pc.model.clone().into_iter().collect(),
            ));
        }
    }

    let client = if clients.is_empty() {
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
    };

    LlmClients {
        client,
        swappable: swappable_clients,
    }
}
