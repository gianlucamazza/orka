use std::{collections::HashMap, path::PathBuf, sync::Arc};

use notify::{EventKind, RecommendedWatcher, RecursiveMode, Watcher};
use orka_core::{config::LlmProviderConfig, traits::SecretManager};
use orka_llm::SwappableLlmClient;
use tokio::{sync::mpsc, task::JoinHandle};
use tracing::{debug, info, warn};

use crate::providers::default_env_var;

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

                    let key = resolve_key_from_env(&env_vars, pc, &*secrets).await;
                    let Some(key) = key else {
                        continue;
                    };

                    if current_keys.get(&pc.name).is_some_and(|p| p == &key) {
                        continue;
                    }
                    current_keys.insert(pc.name.clone(), key.clone());

                    let new_client: Arc<dyn orka_llm::LlmClient> = match pc.provider.as_str() {
                        "anthropic" => Arc::new(orka_llm::AnthropicClient::with_options(
                            key,
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
                    info!(provider = %pc.name, "API key rotated — LLM client swapped");
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
async fn resolve_key_from_env(
    env_vars: &HashMap<String, String>,
    config: &LlmProviderConfig,
    secrets: &dyn SecretManager,
) -> Option<String> {
    // 1. Direct config
    let key = config.api_key.clone().filter(|k| !k.is_empty());
    // 2. Explicit env var name — check file vars first, then process env
    let key = key.or_else(|| {
        config.api_key_env.as_deref().and_then(|env| {
            env_vars
                .get(env)
                .filter(|k| !k.is_empty())
                .cloned()
                .or_else(|| std::env::var(env).ok().filter(|k| !k.is_empty()))
        })
    });
    // 3. Default env var
    let default_env = default_env_var(&config.provider);
    let key = key.or_else(|| {
        if default_env.is_empty() {
            return None;
        }
        env_vars
            .get(default_env)
            .filter(|k| !k.is_empty())
            .cloned()
            .or_else(|| std::env::var(default_env).ok().filter(|k| !k.is_empty()))
    });
    // 4. Secret store
    if key.is_some() {
        return key;
    }
    if let Some(key_name) = config.api_key_secret.as_deref()
        && let Ok(s) = secrets.get_secret(key_name).await
    {
        let k = s.expose_str().unwrap_or("").to_string();
        if !k.is_empty() {
            return Some(k);
        }
    }
    None
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
