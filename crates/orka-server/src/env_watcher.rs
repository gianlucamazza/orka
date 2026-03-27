use std::{collections::HashMap, path::PathBuf, sync::Arc};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use orka_core::{
    config::{LlmAuthKind, LlmProviderConfig},
    traits::SecretManager,
};
use orka_llm::SwappableLlmClient;
use tokio::{sync::mpsc, task::JoinHandle};
use tracing::{debug, info, warn};

use crate::providers::{default_auth_token_env_var, default_env_var};

/// Watches an `.env` file for changes and hot-swaps LLM clients when API keys
/// rotate.
pub(crate) struct EnvWatcher {
    _watcher: RecommendedWatcher,
    _handle: JoinHandle<()>,
}

impl EnvWatcher {
    /// Start watching the env file. Returns `None` if no env file is found.
    pub(crate) fn start(
        providers: Vec<LlmProviderConfig>,
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

                    if current_keys.get(&pc.name).is_some_and(|p| p == &key) {
                        continue;
                    }
                    current_keys.insert(pc.name.clone(), key.clone());

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
                            pc.model
                                .clone()
                                .unwrap_or_else(|| "claude-3-5-sonnet-latest".into()),
                            pc.timeout_secs
                                .unwrap_or(orka_core::config::defaults::default_llm_timeout_secs()),
                            pc.max_tokens
                                .unwrap_or(orka_core::config::defaults::default_llm_max_tokens()),
                            pc.max_retries
                                .unwrap_or(orka_core::config::defaults::default_llm_max_retries()),
                            orka_llm::ANTHROPIC_API_VERSION.into(),
                            pc.base_url.clone(),
                        )),
                        "openai" => {
                            let url = pc
                                .base_url
                                .clone()
                                .unwrap_or_else(|| "https://api.openai.com/v1".into());
                            Arc::new(orka_llm::OpenAiClient::with_options(
                                key,
                                pc.model.clone().unwrap_or_else(|| "gpt-4.1-mini".into()),
                                pc.timeout_secs.unwrap_or(30),
                                pc.max_tokens.unwrap_or(8192),
                                pc.max_retries.unwrap_or(2),
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
    value: String,
    auth_kind: LlmAuthKind,
}

fn looks_like_anthropic_bearer_token(value: &str) -> bool {
    value.starts_with("sk-ant-oat")
}

fn resolve_env_slot(
    env_vars: &HashMap<String, String>,
    provider: &str,
    auth_slot: bool,
    config: &LlmProviderConfig,
) -> Option<String> {
    let (inline, env_name, default_env) = if auth_slot {
        (
            config.auth_token.as_ref(),
            config.auth_token_env.as_deref(),
            default_auth_token_env_var(provider),
        )
    } else {
        (
            config.api_key.as_ref(),
            config.api_key_env.as_deref(),
            default_env_var(provider),
        )
    };

    inline
        .clone()
        .filter(|k| !k.is_empty())
        .or_else(|| {
            env_name.and_then(|env| {
                env_vars
                    .get(env)
                    .filter(|k| !k.is_empty())
                    .cloned()
                    .or_else(|| std::env::var(env).ok().filter(|k| !k.is_empty()))
            })
        })
        .or_else(|| {
            if default_env.is_empty() {
                return None;
            }
            env_vars
                .get(default_env)
                .filter(|k| !k.is_empty())
                .cloned()
                .or_else(|| std::env::var(default_env).ok().filter(|k| !k.is_empty()))
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
                value,
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
                value,
                auth_kind: config.auth_kind,
            })
        }
        LlmAuthKind::Cli => None,
        LlmAuthKind::Auto => {
            if let Some(value) = resolve_env_slot(env_vars, &config.provider, true, config) {
                return Some(ResolvedEnvCredential {
                    value,
                    auth_kind: LlmAuthKind::AuthToken,
                });
            }
            let mut value = resolve_env_slot(env_vars, &config.provider, false, config);
            if value.is_none() {
                value = secret_value(secrets, config.api_key_secret.as_deref()).await;
            }
            let value = value?;
            let auth_kind =
                if config.provider == "anthropic" && looks_like_anthropic_bearer_token(&value) {
                    LlmAuthKind::AuthToken
                } else {
                    LlmAuthKind::ApiKey
                };
            Some(ResolvedEnvCredential { value, auth_kind })
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
