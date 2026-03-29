use std::{collections::HashMap, path::PathBuf, sync::Arc};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use orka_config::{LlmAuthKind, LlmProviderConfig, defaults};
use orka_core::{SecretStr, traits::SecretManager};
use orka_llm::SwappableLlmClient;
use tokio::{sync::mpsc, task::JoinHandle};
use tracing::{debug, info, warn};

use crate::providers::{default_auth_token_env_var, default_base_url, default_env_var};

/// Watches an `.env` file for changes and hot-swaps LLM clients when API keys
/// rotate.
pub(crate) struct EnvWatcher {
    _watcher: RecommendedWatcher,
    _handle: JoinHandle<()>,
}

impl EnvWatcher {
    /// Start watching the env file. Returns `None` if no env file is found.
    #[allow(clippy::too_many_lines)]
    pub(crate) fn start(
        providers: Vec<LlmProviderConfig>,
        default_model: String,
        clients: HashMap<String, Arc<SwappableLlmClient>>,
        secrets: Arc<dyn SecretManager>,
    ) -> Option<Self> {
        let env_path = resolve_env_path()?;
        info!(path = %env_path.display(), "watching env file for secret rotation");

        let (tx, mut rx) = mpsc::channel::<notify::Event>(64);

        // Watch the parent directory (some editors do atomic rename)
        let watch_dir = env_path.parent()?.to_path_buf();
        let mut watcher = RecommendedWatcher::new(
            move |res: Result<notify::Event, notify::Error>| {
                if let Ok(event) = res {
                    let _ = tx.blocking_send(event);
                }
            },
            notify::Config::default(),
        )
        .ok()?;

        watcher
            .watch(&watch_dir, RecursiveMode::NonRecursive)
            .ok()?;

        let handle = tokio::spawn(async move {
            use tokio::time::{Duration, Instant};

            let debounce = Duration::from_millis(500);
            let mut last_reload = Instant::now() - debounce;
            let mut current_keys: HashMap<String, String> = HashMap::new();

            while let Some(event) = rx.recv().await {
                if !matches!(event.kind, EventKind::Modify(_) | EventKind::Create(_)) {
                    continue;
                }
                if !event.paths.iter().any(|p| p == &env_path) {
                    continue;
                }
                let now = Instant::now();
                if now.duration_since(last_reload) < debounce {
                    continue;
                }
                last_reload = now;

                debug!(path = %env_path.display(), "env file changed, checking for key rotation");

                // Parse env file into a map without polluting process environment
                let env_vars: HashMap<String, String> = match dotenvy::from_path_iter(&env_path) {
                    Ok(iter) => iter.filter_map(std::result::Result::ok).collect(),
                    Err(e) => {
                        warn!(%e, "failed to parse env file");
                        continue;
                    }
                };

                for pc in &providers {
                    let Some(swappable) = clients.get(&pc.name) else {
                        continue;
                    };

                    let credential = resolve_credential_from_env(&env_vars, pc, &*secrets).await;
                    let Some(credential) = credential else {
                        continue;
                    };
                    let key = credential.value;

                    if current_keys
                        .get(&pc.name)
                        .is_some_and(|p| p == key.expose())
                    {
                        continue;
                    }
                    current_keys.insert(pc.name.clone(), key.expose().to_string());

                    let new_client: Arc<dyn orka_llm::LlmClient> = match pc.provider.as_str() {
                        "anthropic" => Arc::new(orka_llm::AnthropicClient::with_auth_options(
                            key,
                            match credential.auth_kind {
                                LlmAuthKind::ApiKey => orka_llm::AnthropicAuthKind::ApiKey,
                                LlmAuthKind::AuthToken | LlmAuthKind::Subscription => {
                                    orka_llm::AnthropicAuthKind::Bearer
                                }
                                _ => orka_llm::AnthropicAuthKind::Auto,
                            },
                            resolved_model(pc, &default_model),
                            pc.timeout_secs
                                .unwrap_or(defaults::default_llm_timeout_secs()),
                            pc.max_tokens.unwrap_or(defaults::default_llm_max_tokens()),
                            pc.max_retries
                                .unwrap_or(defaults::default_llm_max_retries()),
                            orka_llm::ANTHROPIC_API_VERSION.into(),
                            pc.base_url.clone(),
                        )),
                        "openai" | "moonshot" => {
                            let url = pc.base_url.clone().unwrap_or_else(|| {
                                default_base_url(&pc.provider)
                                    .unwrap_or(crate::providers::OPENAI_BASE_URL)
                                    .to_string()
                            });
                            Arc::new(orka_llm::OpenAiClient::with_options(
                                key,
                                resolved_model(pc, &default_model),
                                pc.timeout_secs
                                    .unwrap_or(defaults::default_llm_timeout_secs()),
                                pc.max_tokens.unwrap_or(defaults::default_llm_max_tokens()),
                                pc.max_retries
                                    .unwrap_or(defaults::default_llm_max_retries()),
                                url,
                            ))
                        }
                        _ => continue,
                    };

                    swappable.swap(new_client);
                    info!(
                        provider = %pc.name,
                        auth_kind = ?credential.auth_kind,
                        "LLM credential rotated — client swapped"
                    );
                }
            }
        });

        Some(Self {
            _watcher: watcher,
            _handle: handle,
        })
    }
}

/// Resolve an API key from parsed env file vars + secret store (no process env
/// mutation).
///
/// Fallback order:
///   1. `api_key` in config (direct)
///   2. `api_key_env` looked up in `env_vars`
///   3. Default env var (e.g. `ANTHROPIC_API_KEY`) looked up in `env_vars`,
///      then process env
///   4. Secret store
struct ResolvedEnvCredential {
    value: SecretStr,
    auth_kind: LlmAuthKind,
}

fn resolved_model(provider: &LlmProviderConfig, default_model: &str) -> String {
    provider
        .model
        .clone()
        .unwrap_or_else(|| default_model.to_string())
}

fn looks_like_anthropic_bearer_token(value: &str) -> bool {
    value.starts_with("sk-ant-oat")
}

fn inferred_auth_kind(provider: &str, value: &str) -> LlmAuthKind {
    if provider == "anthropic" && looks_like_anthropic_bearer_token(value) {
        LlmAuthKind::AuthToken
    } else {
        LlmAuthKind::ApiKey
    }
}

fn resolve_env_slot(
    env_vars: &HashMap<String, String>,
    provider: &str,
    auth_slot: bool,
    config: &LlmProviderConfig,
) -> Option<String> {
    let (inline, env_name, default_env) = if auth_slot {
        (
            config.auth_token.as_deref(),
            config.auth_token_env.as_deref(),
            default_auth_token_env_var(provider),
        )
    } else {
        (
            config.api_key.as_deref(),
            config.api_key_env.as_deref(),
            default_env_var(provider),
        )
    };

    inline
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
        .or_else(|| {
            env_name.and_then(|env| {
                env_vars
                    .get(env)
                    .map(String::as_str)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned)
                    .or_else(|| std::env::var(env).ok().filter(|value| !value.is_empty()))
            })
        })
        .or_else(|| {
            (!default_env.is_empty())
                .then_some(default_env)
                .and_then(|env| {
                    env_vars
                        .get(env)
                        .map(String::as_str)
                        .filter(|value| !value.is_empty())
                        .map(str::to_owned)
                        .or_else(|| std::env::var(env).ok().filter(|value| !value.is_empty()))
                })
        })
}

async fn resolve_credential_from_env(
    env_vars: &HashMap<String, String>,
    config: &LlmProviderConfig,
    secrets: &dyn SecretManager,
) -> Option<ResolvedEnvCredential> {
    async fn secret_value(secrets: &dyn SecretManager, path: Option<&str>) -> Option<String> {
        let path = path?;
        let secret = secrets.get_secret(path).await.ok()?;
        let value = secret.expose_str().unwrap_or("").to_string();
        (!value.is_empty()).then_some(value)
    }

    match config.auth_kind {
        LlmAuthKind::ApiKey => {
            let mut value = resolve_env_slot(env_vars, &config.provider, false, config);
            if value.is_none() {
                value = secret_value(secrets, config.api_key_secret.as_deref()).await;
            }
            let value = value?;
            Some(ResolvedEnvCredential {
                value: SecretStr::new(value),
                auth_kind: LlmAuthKind::ApiKey,
            })
        }
        LlmAuthKind::AuthToken | LlmAuthKind::Subscription => {
            let mut value = resolve_env_slot(env_vars, &config.provider, true, config)
                .or_else(|| resolve_env_slot(env_vars, &config.provider, false, config));
            if value.is_none() {
                value = secret_value(secrets, config.auth_token_secret.as_deref()).await;
            }
            if value.is_none() {
                value = secret_value(secrets, config.api_key_secret.as_deref()).await;
            }
            let value = value?;
            Some(ResolvedEnvCredential {
                value: SecretStr::new(value),
                auth_kind: config.auth_kind,
            })
        }
        LlmAuthKind::Cli => None,
        LlmAuthKind::Auto => {
            if let Some(value) = resolve_env_slot(env_vars, &config.provider, true, config) {
                return Some(ResolvedEnvCredential {
                    value: SecretStr::new(value),
                    auth_kind: LlmAuthKind::AuthToken,
                });
            }
            let mut value = resolve_env_slot(env_vars, &config.provider, false, config);
            if value.is_none() {
                value = secret_value(secrets, config.api_key_secret.as_deref()).await;
            }
            let value = value?;
            let auth_kind = inferred_auth_kind(&config.provider, &value);
            Some(ResolvedEnvCredential {
                value: SecretStr::new(value),
                auth_kind,
            })
        }
        _ => {
            let mut value = resolve_env_slot(env_vars, &config.provider, true, config)
                .or_else(|| resolve_env_slot(env_vars, &config.provider, false, config));
            if value.is_none() {
                value = secret_value(secrets, config.auth_token_secret.as_deref()).await;
            }
            if value.is_none() {
                value = secret_value(secrets, config.api_key_secret.as_deref()).await;
            }
            let value = value?;
            let auth_kind = inferred_auth_kind(&config.provider, &value);
            Some(ResolvedEnvCredential {
                value: SecretStr::new(value),
                auth_kind,
            })
        }
    }
}

/// Resolve the env file path to watch.
///
/// 1. `ORKA_ENV_FILE` env var (explicit path)
/// 2. `/etc/orka/orka.env` (production)
/// 3. `.env` in CWD (dev)
fn resolve_env_path() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("ORKA_ENV_FILE") {
        let path = PathBuf::from(p);
        if path.exists() {
            return Some(path);
        }
    }

    let prod = PathBuf::from("/etc/orka/orka.env");
    if prod.exists() {
        return Some(prod);
    }

    let dev = PathBuf::from(".env");
    if dev.exists() {
        return Some(dev);
    }

    None
}

#[cfg(test)]
#[allow(clippy::expect_used)]
mod tests {
    use orka_config::{LlmAuthKind, LlmProviderConfig};
    use orka_core::{SecretValue, traits::SecretManager};

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
        LlmProviderConfig::for_provider("anthropic", "anthropic")
    }

    #[test]
    fn resolved_model_uses_global_default_when_provider_model_is_missing() {
        let provider = LlmProviderConfig::for_provider("moonshot", "moonshot");
        assert_eq!(
            super::resolved_model(&provider, "global-default"),
            "global-default"
        );
    }

    #[test]
    fn resolve_env_slot_prefers_inline_value() {
        let mut provider = provider();
        provider.api_key = Some("inline-key".into());

        let resolved = resolve_env_slot(&HashMap::new(), "anthropic", false, &provider);
        assert_eq!(resolved.as_deref(), Some("inline-key"));
    }

    #[test]
    fn resolve_env_slot_reads_named_env_var_from_env_file_map() {
        let mut provider = provider();
        provider.auth_token_env = Some("CUSTOM_AUTH".into());

        let env_vars = HashMap::from([(String::from("CUSTOM_AUTH"), String::from("token"))]);
        let resolved = resolve_env_slot(&env_vars, "anthropic", true, &provider);
        assert_eq!(resolved.as_deref(), Some("token"));
    }

    #[test]
    fn resolve_env_slot_reads_default_env_var_from_env_file_map() {
        let env_vars = HashMap::from([(
            String::from("ANTHROPIC_API_KEY"),
            String::from("default-key"),
        )]);
        let resolved = resolve_env_slot(&env_vars, "anthropic", false, &provider());
        assert_eq!(resolved.as_deref(), Some("default-key"));
    }

    #[test]
    fn resolve_env_slot_reads_moonshot_default_env_var_from_env_file_map() {
        let env_vars = HashMap::from([(
            String::from("MOONSHOT_API_KEY"),
            String::from("moonshot-key"),
        )]);
        let provider = LlmProviderConfig::for_provider("moonshot", "moonshot");
        let resolved = resolve_env_slot(&env_vars, "moonshot", false, &provider);
        assert_eq!(resolved.as_deref(), Some("moonshot-key"));
    }

    #[tokio::test]
    async fn cli_auth_kind_skips_env_resolution() {
        let mut provider = provider();
        provider.auth_kind = LlmAuthKind::Cli;
        provider.api_key = Some("ignored".into());

        let resolved = resolve_credential_from_env(&HashMap::new(), &provider, &NoopSecrets).await;
        assert!(resolved.is_none());
    }

    #[tokio::test]
    async fn auto_auth_kind_detects_anthropic_bearer_tokens() {
        let mut provider = provider();
        provider.auth_kind = LlmAuthKind::Auto;
        provider.api_key = Some("sk-ant-oat01-test".into());

        let resolved = resolve_credential_from_env(&HashMap::new(), &provider, &NoopSecrets).await;
        let resolved = resolved.expect("credential");
        assert_eq!(resolved.auth_kind, LlmAuthKind::AuthToken);
        assert_eq!(resolved.value.expose(), "sk-ant-oat01-test");
    }
}
