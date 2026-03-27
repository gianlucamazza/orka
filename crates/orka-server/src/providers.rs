//! LLM provider resolution: credential lookup and client construction.

use std::{collections::HashMap, sync::Arc};

use orka_core::{
    config::{LlmAuthKind, LlmProviderConfig, OrkaConfig},
    traits::SecretManager,
};
use orka_llm::{AnthropicAuthKind, SwappableLlmClient};
use tracing::{info, warn};

/// Default environment variable name for a provider's API key.
pub(crate) fn default_env_var(provider: &str) -> &str {
    match provider {
        "anthropic" => "ANTHROPIC_API_KEY",
        "openai" => "OPENAI_API_KEY",
        _ => "",
    }
}

/// Default environment variable name for a provider's bearer/auth token.
pub(crate) fn default_auth_token_env_var(provider: &str) -> &str {
    match provider {
        "anthropic" => "ANTHROPIC_AUTH_TOKEN",
        _ => "",
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CredentialSlot {
    ApiKey,
    AuthToken,
}

#[derive(Debug, Clone)]
pub(crate) struct ResolvedCredential {
    value: String,
    source: String,
    slot: CredentialSlot,
    auth_kind: LlmAuthKind,
}

fn looks_like_anthropic_bearer_token(value: &str) -> bool {
    value.starts_with("sk-ant-oat")
}

async fn resolve_slot(
    provider: &str,
    slot: CredentialSlot,
    config: &LlmProviderConfig,
    secrets: &dyn SecretManager,
) -> Option<(String, String)> {
    let (inline, env_name, secret_path, default_env) = match slot {
        CredentialSlot::ApiKey => (
            config.api_key.as_deref(),
            config.api_key_env.as_deref(),
            config.api_key_secret.as_deref(),
            default_env_var(provider),
        ),
        CredentialSlot::AuthToken => (
            config.auth_token.as_deref(),
            config.auth_token_env.as_deref(),
            config.auth_token_secret.as_deref(),
            default_auth_token_env_var(provider),
        ),
    };

    if let Some(value) = inline.filter(|value| !value.is_empty()) {
        let label = match slot {
            CredentialSlot::ApiKey => "inline api_key",
            CredentialSlot::AuthToken => "inline auth_token",
        };
        return Some((value.to_string(), label.into()));
    }

    if let Some(env) = env_name
        && let Ok(value) = std::env::var(env)
        && !value.is_empty()
    {
        let label = match slot {
            CredentialSlot::ApiKey => format!("env:{env}"),
            CredentialSlot::AuthToken => format!("auth env:{env}"),
        };
        return Some((value, label));
    }

    if !default_env.is_empty()
        && let Ok(value) = std::env::var(default_env)
        && !value.is_empty()
    {
        let label = match slot {
            CredentialSlot::ApiKey => format!("env:{default_env} (default)"),
            CredentialSlot::AuthToken => format!("auth env:{default_env} (default)"),
        };
        return Some((value, label));
    }

    if let Some(path) = secret_path {
        match secrets.get_secret(path).await {
            Ok(secret) => {
                let value = secret.expose_str().unwrap_or("").to_string();
                if !value.is_empty() {
                    let label = match slot {
                        CredentialSlot::ApiKey => format!("secret:{path}"),
                        CredentialSlot::AuthToken => format!("auth secret:{path}"),
                    };
                    return Some((value, label));
                }
            }
            Err(error) => {
                tracing::debug!(provider, path, %error, "failed to read credential from secret store");
            }
        }
    }

    None
}

/// Resolve the effective runtime credential for a provider.
pub(crate) async fn resolve_llm_credential(
    provider: &str,
    config: &LlmProviderConfig,
    secrets: &dyn SecretManager,
) -> Option<ResolvedCredential> {
    match config.auth_kind {
        LlmAuthKind::ApiKey => {
            let (value, source) =
                resolve_slot(provider, CredentialSlot::ApiKey, config, secrets).await?;
            if provider == "anthropic" && looks_like_anthropic_bearer_token(&value) {
                warn!(
                    provider = config.name,
                    source = %source,
                    "auth_kind=api_key but credential looks like an Anthropic bearer/setup-token"
                );
            }
            Some(ResolvedCredential {
                value,
                source,
                slot: CredentialSlot::ApiKey,
                auth_kind: LlmAuthKind::ApiKey,
            })
        }
        LlmAuthKind::AuthToken | LlmAuthKind::Subscription => {
            let resolved = if let Some((value, source)) =
                resolve_slot(provider, CredentialSlot::AuthToken, config, secrets).await
            {
                (value, source)
            } else {
                let (value, source) =
                    resolve_slot(provider, CredentialSlot::ApiKey, config, secrets).await?;
                (value, format!("{source} (legacy auth-token fallback)"))
            };

            let (value, source) = resolved;
            if provider == "anthropic" && !looks_like_anthropic_bearer_token(&value) {
                warn!(
                    provider = config.name,
                    source = %source,
                    "auth_kind expects bearer semantics but credential does not look like a Claude setup-token/bearer token"
                );
            }
            Some(ResolvedCredential {
                value,
                source,
                slot: CredentialSlot::AuthToken,
                auth_kind: config.auth_kind,
            })
        }
        LlmAuthKind::Cli => None,
        LlmAuthKind::Auto => {
            if let Some((value, source)) =
                resolve_slot(provider, CredentialSlot::AuthToken, config, secrets).await
            {
                return Some(ResolvedCredential {
                    value,
                    source,
                    slot: CredentialSlot::AuthToken,
                    auth_kind: LlmAuthKind::AuthToken,
                });
            }

            let (value, source) =
                resolve_slot(provider, CredentialSlot::ApiKey, config, secrets).await?;
            let auth_kind = if provider == "anthropic" && looks_like_anthropic_bearer_token(&value)
            {
                LlmAuthKind::AuthToken
            } else {
                LlmAuthKind::ApiKey
            };
            let slot = match auth_kind {
                LlmAuthKind::AuthToken => CredentialSlot::AuthToken,
                _ => CredentialSlot::ApiKey,
            };
            Some(ResolvedCredential {
                value,
                source,
                slot,
                auth_kind,
            })
        }
    }
}

fn anthropic_auth_kind(resolved: &ResolvedCredential) -> AnthropicAuthKind {
    match resolved.auth_kind {
        LlmAuthKind::ApiKey => AnthropicAuthKind::ApiKey,
        LlmAuthKind::AuthToken | LlmAuthKind::Subscription => AnthropicAuthKind::Bearer,
        LlmAuthKind::Auto => AnthropicAuthKind::Auto,
        LlmAuthKind::Cli => AnthropicAuthKind::ApiKey,
    }
}

fn build_anthropic_client(
    config: &OrkaConfig,
    provider: &LlmProviderConfig,
    resolved: ResolvedCredential,
) -> Arc<dyn orka_llm::LlmClient> {
    let model = provider
        .model
        .clone()
        .unwrap_or_else(|| config.llm.default_model.clone());
    Arc::new(orka_llm::AnthropicClient::with_auth_options(
        resolved.value,
        anthropic_auth_kind(&resolved),
        model,
        provider
            .timeout_secs
            .unwrap_or(orka_core::config::defaults::default_llm_timeout_secs()),
        provider
            .max_tokens
            .unwrap_or(orka_core::config::defaults::default_llm_max_tokens()),
        provider
            .max_retries
            .unwrap_or(orka_core::config::defaults::default_llm_max_retries()),
        orka_llm::ANTHROPIC_API_VERSION.into(),
        provider.base_url.clone(),
    )) as Arc<dyn orka_llm::LlmClient>
}

/// Result of building all LLM clients from config.
pub(crate) struct LlmClients {
    /// The composed LLM client (router or single), ready to use.
    pub client: Option<Arc<dyn orka_llm::LlmClient>>,
    /// Swappable wrappers keyed by provider name, used for hot-reload.
    pub swappable: HashMap<String, Arc<SwappableLlmClient>>,
}

/// Build the LLM client(s) from the config, resolving credentials from env and
/// the secret store.
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
            "anthropic" => match pc.auth_kind {
                LlmAuthKind::Cli => {
                    warn!(
                        provider = %pc.name,
                        "llm.providers auth_kind=cli is not supported yet; use os.coding.providers.claude_code for Claude CLI integration"
                    );
                    None
                }
                _ => resolve_llm_credential("anthropic", pc, secrets)
                    .await
                    .map(|credential| {
                        info!(
                            provider = %pc.name,
                            model = ?pc.model,
                            auth_kind = ?credential.auth_kind,
                            source = %credential.source,
                            "LLM provider credential resolved"
                        );
                        build_anthropic_client(config, pc, credential)
                    }),
            },
            "openai" => {
                let credential = resolve_llm_credential("openai", pc, secrets).await;
                credential.map(|resolved| {
                    if resolved.slot == CredentialSlot::AuthToken {
                        warn!(
                            provider = %pc.name,
                            "resolved bearer-style auth for OpenAI provider; falling back to API-key semantics"
                        );
                    }
                    let model = pc
                        .model
                        .clone()
                        .unwrap_or_else(|| config.llm.default_model.clone());
                    let url = pc
                        .base_url
                        .clone()
                        .unwrap_or_else(|| "https://api.openai.com/v1".into());
                    Arc::new(orka_llm::OpenAiClient::with_options(
                        resolved.value,
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

#[cfg(test)]
mod tests {
    use orka_core::{
        SecretValue,
        config::{LlmAuthKind, LlmProviderConfig},
        traits::SecretManager,
    };

    use super::*;

    struct NoopSecrets;

    #[async_trait::async_trait]
    impl SecretManager for NoopSecrets {
        async fn get_secret(&self, _: &str) -> orka_core::Result<SecretValue> {
            Err(orka_core::Error::secret("missing"))
        }
        async fn set_secret(&self, _: &str, _: &SecretValue) -> orka_core::Result<()> {
            Ok(())
        }
        async fn delete_secret(&self, _: &str) -> orka_core::Result<()> {
            Ok(())
        }
        async fn list_secrets(&self) -> orka_core::Result<Vec<String>> {
            Ok(Vec::new())
        }
    }

    fn provider() -> LlmProviderConfig {
        LlmProviderConfig {
            name: "anthropic".into(),
            provider: "anthropic".into(),
            ..LlmProviderConfig::default()
        }
    }

    #[tokio::test]
    async fn auto_detects_bearer_token_from_legacy_api_key_field() {
        let mut provider = provider();
        provider.api_key = Some("sk-ant-oat01-test".into());
        let resolved = resolve_llm_credential("anthropic", &provider, &NoopSecrets)
            .await
            .expect("credential");
        assert_eq!(resolved.auth_kind, LlmAuthKind::AuthToken);
        assert_eq!(resolved.slot, CredentialSlot::AuthToken);
    }

    #[tokio::test]
    async fn explicit_auth_token_prefers_auth_token_sources() {
        let mut provider = provider();
        provider.auth_kind = LlmAuthKind::AuthToken;
        provider.auth_token = Some("sk-ant-oat01-token".into());
        provider.api_key = Some("sk-ant-api03-key".into());
        let resolved = resolve_llm_credential("anthropic", &provider, &NoopSecrets)
            .await
            .expect("credential");
        assert_eq!(resolved.auth_kind, LlmAuthKind::AuthToken);
        assert_eq!(resolved.value, "sk-ant-oat01-token");
    }

    #[tokio::test]
    async fn explicit_auth_token_falls_back_to_legacy_api_key_sources() {
        let mut provider = provider();
        provider.auth_kind = LlmAuthKind::Subscription;
        provider.api_key = Some("sk-ant-oat01-legacy".into());
        let resolved = resolve_llm_credential("anthropic", &provider, &NoopSecrets)
            .await
            .expect("credential");
        assert_eq!(resolved.auth_kind, LlmAuthKind::Subscription);
        assert_eq!(resolved.slot, CredentialSlot::AuthToken);
    }

    #[test]
    fn default_auth_token_env_var_is_available_for_anthropic() {
        assert_eq!(
            default_auth_token_env_var("anthropic"),
            "ANTHROPIC_AUTH_TOKEN"
        );
        assert_eq!(default_auth_token_env_var("openai"), "");
    }
}
