use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use orka_circuit_breaker::{CircuitBreaker, CircuitBreakerConfig, CircuitBreakerError};
use orka_core::Result;
use tracing::{debug, warn};

use crate::client::{
    ChatMessage, CompletionOptions, CompletionResponse, LlmClient, LlmStream, LlmToolStream,
    ToolDefinition,
};

/// Routes LLM requests to the appropriate provider based on model name prefix.
///
/// Each provider is protected by a circuit breaker that trips on consecutive
/// failures (default: 5 failures, 30s open duration).
///
/// When a provider's circuit breaker is open, the router can optionally try
/// a list of fallback providers in order before returning an error.
pub struct LlmRouter {
    /// Default provider used when no prefix matches.
    default_provider: Arc<dyn LlmClient>,
    /// Map of provider name -> client (e.g., "anthropic" -> `AnthropicClient`).
    providers: HashMap<String, Arc<dyn LlmClient>>,
    /// Map of model prefix -> provider name (e.g., "claude" -> "anthropic",
    /// "gpt" -> "openai").
    prefix_map: HashMap<String, String>,
    /// Per-provider circuit breakers.
    breakers: HashMap<String, Arc<CircuitBreaker>>,
    /// Circuit breaker for the default provider.
    default_breaker: Arc<CircuitBreaker>,
    /// Circuit breaker config used for new providers.
    breaker_config: CircuitBreakerConfig,
    /// Ordered list of provider names to try when the primary provider's
    /// circuit breaker is open.  Empty means no fallback (default).
    fallback_providers: Vec<String>,
}

impl LlmRouter {
    /// Create a router with a default (fallback) provider.
    pub fn new(default_provider: Arc<dyn LlmClient>) -> Self {
        let config = CircuitBreakerConfig::default();
        let default_breaker = Arc::new(CircuitBreaker::new(config.clone()));
        Self {
            default_provider,
            providers: HashMap::new(),
            prefix_map: HashMap::new(),
            breakers: HashMap::new(),
            default_breaker,
            breaker_config: config,
            fallback_providers: Vec::new(),
        }
    }

    /// Set an ordered list of provider names to try when the primary
    /// provider's circuit breaker is open.
    ///
    /// When the resolved provider is unavailable, the router tries each
    /// fallback in order and returns the first successful response.
    #[must_use]
    pub fn with_fallback_providers(mut self, providers: Vec<String>) -> Self {
        self.fallback_providers = providers;
        self
    }

    /// Set a custom circuit breaker config. Affects subsequently added
    /// providers and replaces the default provider's breaker.
    #[must_use]
    pub fn with_circuit_breaker_config(mut self, config: CircuitBreakerConfig) -> Self {
        self.default_breaker = Arc::new(CircuitBreaker::new(config.clone()));
        self.breaker_config = config;
        self
    }

    /// Register a provider with model-name prefixes for routing.
    #[must_use]
    pub fn add_provider(
        mut self,
        name: impl Into<String>,
        client: Arc<dyn LlmClient>,
        prefixes: Vec<String>,
    ) -> Self {
        let name = name.into();
        self.providers.insert(name.clone(), client);
        self.breakers.insert(
            name.clone(),
            Arc::new(CircuitBreaker::new(self.breaker_config.clone())),
        );
        for prefix in prefixes {
            self.prefix_map.insert(prefix, name.clone());
        }
        self
    }

    fn resolve(&self, model: Option<&str>) -> (&dyn LlmClient, &Arc<CircuitBreaker>) {
        if let Some(model_name) = model {
            // Longest-prefix-match: among all registered prefixes that are a
            // prefix of `model_name`, pick the most specific one (longest).
            // This guarantees deterministic routing even when multiple prefixes
            // overlap (e.g. "gpt" and "gpt-4o").
            let best_match = self
                .prefix_map
                .iter()
                .filter(|(prefix, _)| model_name.starts_with(prefix.as_str()))
                .max_by_key(|(prefix, _)| prefix.len());

            if let Some((_, provider_name)) = best_match
                && let Some(client) = self.providers.get(provider_name)
            {
                debug!(
                    model = model_name,
                    provider = provider_name,
                    "routing to provider"
                );
                let breaker = self
                    .breakers
                    .get(provider_name)
                    .unwrap_or(&self.default_breaker);
                return (client.as_ref(), breaker);
            }

            // Exact match on provider name (model name == provider name).
            if let Some(client) = self.providers.get(model_name) {
                let breaker = self
                    .breakers
                    .get(model_name)
                    .unwrap_or(&self.default_breaker);
                return (client.as_ref(), breaker);
            }
        }
        (self.default_provider.as_ref(), &self.default_breaker)
    }
}

/// Map a circuit breaker error to an orka Error.
fn map_cb_err(e: CircuitBreakerError<orka_core::Error>) -> orka_core::Error {
    match e {
        CircuitBreakerError::Open => {
            warn!("LLM circuit breaker is open — request rejected immediately");
            orka_core::Error::llm_msg(
                "LLM provider is temporarily unavailable (circuit breaker open). Please retry later.",
            )
        }
        CircuitBreakerError::Inner(inner) => inner,
        _ => orka_core::Error::llm_msg("unknown circuit breaker error"),
    }
}

/// Returns `true` if the error indicates the circuit breaker was open (as
/// opposed to an actual call failure).
fn is_cb_open<E>(e: &CircuitBreakerError<E>) -> bool {
    matches!(e, CircuitBreakerError::Open)
}

#[async_trait]
impl LlmClient for LlmRouter {
    async fn complete(&self, messages: Vec<ChatMessage>, system: &str) -> Result<String> {
        let provider = self.default_provider.clone();
        let system = system.to_string();
        self.default_breaker
            .call(|| {
                let provider = provider.clone();
                let messages = messages.clone();
                let system = system.clone();
                async move { provider.complete(messages, &system).await }
            })
            .await
            .map_err(map_cb_err)
    }

    async fn complete_with_options(
        &self,
        messages: Vec<ChatMessage>,
        system: &str,
        options: &CompletionOptions,
    ) -> Result<String> {
        let candidates = self.ordered_providers(options.model.as_deref());
        let system = system.to_string();
        let options = options.clone();

        let mut last_err = None;
        for (provider, breaker) in candidates {
            let result = breaker
                .call(|| {
                    let provider = provider.clone();
                    let messages = messages.clone();
                    let system = system.clone();
                    let options = options.clone();
                    async move {
                        provider
                            .complete_with_options(messages, &system, &options)
                            .await
                    }
                })
                .await;

            match result {
                Ok(r) => return Ok(r),
                Err(ref e) if is_cb_open(e) => {
                    warn!("LLM provider circuit breaker open, trying next fallback");
                    last_err = Some(result.map(|_| unreachable!()).map_err(map_cb_err));
                }
                Err(e) => return Err(map_cb_err(e)),
            }
        }
        last_err.unwrap_or_else(|| {
            Err(orka_core::Error::llm_msg("all LLM providers unavailable"))
        })
    }

    async fn complete_stream(&self, messages: Vec<ChatMessage>, system: &str) -> Result<LlmStream> {
        let provider = self.default_provider.clone();
        let system = system.to_string();
        self.default_breaker
            .call(|| {
                let provider = provider.clone();
                let messages = messages.clone();
                let system = system.clone();
                async move { provider.complete_stream(messages, &system).await }
            })
            .await
            .map_err(map_cb_err)
    }

    async fn complete_with_tools(
        &self,
        messages: &[ChatMessage],
        system: &str,
        tools: &[ToolDefinition],
        options: &CompletionOptions,
    ) -> Result<CompletionResponse> {
        let candidates = self.ordered_providers(options.model.as_deref());
        let system = system.to_string();
        let tools = tools.to_vec();
        let messages = messages.to_vec();
        let options = options.clone();

        let mut last_err = None;
        for (provider, breaker) in candidates {
            let result = breaker
                .call(|| {
                    let provider = provider.clone();
                    let messages = messages.clone();
                    let system = system.clone();
                    let tools = tools.clone();
                    let options = options.clone();
                    async move {
                        provider
                            .complete_with_tools(&messages, &system, &tools, &options)
                            .await
                    }
                })
                .await;

            match result {
                Ok(r) => return Ok(r),
                Err(ref e) if is_cb_open(e) => {
                    warn!("LLM provider circuit breaker open, trying next fallback");
                    last_err = Some(result.map(|_| unreachable!()).map_err(map_cb_err));
                }
                Err(e) => return Err(map_cb_err(e)),
            }
        }
        last_err.unwrap_or_else(|| {
            Err(orka_core::Error::llm_msg("all LLM providers unavailable"))
        })
    }

    async fn complete_stream_with_tools(
        &self,
        messages: &[ChatMessage],
        system: &str,
        tools: &[ToolDefinition],
        options: &CompletionOptions,
    ) -> Result<LlmToolStream> {
        let candidates = self.ordered_providers(options.model.as_deref());
        let system = system.to_string();
        let tools = tools.to_vec();
        let messages = messages.to_vec();
        let options = options.clone();

        let mut last_err = None;
        for (provider, breaker) in candidates {
            let result = breaker
                .call(|| {
                    let provider = provider.clone();
                    let messages = messages.clone();
                    let system = system.clone();
                    let tools = tools.clone();
                    let options = options.clone();
                    async move {
                        provider
                            .complete_stream_with_tools(&messages, &system, &tools, &options)
                            .await
                    }
                })
                .await;

            match result {
                Ok(r) => return Ok(r),
                Err(ref e) if is_cb_open(e) => {
                    warn!("LLM provider circuit breaker open, trying next fallback");
                    last_err = Some(result.map(|_| unreachable!()).map_err(map_cb_err));
                }
                Err(e) => return Err(map_cb_err(e)),
            }
        }
        last_err.unwrap_or_else(|| {
            Err(orka_core::Error::llm_msg("all LLM providers unavailable"))
        })
    }
}

impl LlmRouter {
    /// Build an ordered list of `(provider, breaker)` pairs to try for a
    /// request: primary provider first, then any configured fallbacks.
    fn ordered_providers(
        &self,
        model: Option<&str>,
    ) -> Vec<(Arc<dyn LlmClient>, Arc<CircuitBreaker>)> {
        let primary = self.resolve_provider_arc(model);
        let (_, primary_breaker) = self.resolve(model);
        let mut list = vec![(primary, primary_breaker.clone())];

        for name in &self.fallback_providers {
            if let Some(client) = self.providers.get(name) {
                let breaker = self
                    .breakers
                    .get(name)
                    .cloned()
                    .unwrap_or_else(|| self.default_breaker.clone());
                list.push((client.clone(), breaker));
            }
        }
        list
    }

    /// Resolve the provider as an Arc for use in async closures.
    fn resolve_provider_arc(&self, model: Option<&str>) -> Arc<dyn LlmClient> {
        if let Some(model_name) = model {
            for (prefix, provider_name) in &self.prefix_map {
                if model_name.starts_with(prefix)
                    && let Some(client) = self.providers.get(provider_name)
                {
                    return client.clone();
                }
            }
            if let Some(client) = self.providers.get(model_name) {
                return client.clone();
            }
        }
        self.default_provider.clone()
    }
}

#[cfg(test)]
mod tests {
    #![allow(clippy::unwrap_used)]

    use std::{
        sync::atomic::{AtomicU32, Ordering},
        time::Duration,
    };

    use super::*;

    struct MockLlm {
        fail_count: AtomicU32,
        max_fails: u32,
    }

    impl MockLlm {
        fn new(max_fails: u32) -> Self {
            Self {
                fail_count: AtomicU32::new(0),
                max_fails,
            }
        }
    }

    #[async_trait]
    impl LlmClient for MockLlm {
        async fn complete(&self, _messages: Vec<ChatMessage>, _system: &str) -> Result<String> {
            let count = self.fail_count.fetch_add(1, Ordering::SeqCst);
            if count < self.max_fails {
                Err(orka_core::Error::llm_msg("mock failure"))
            } else {
                Ok("ok".into())
            }
        }
    }

    #[tokio::test]
    async fn circuit_breaker_trips_after_failures() {
        let config = CircuitBreakerConfig {
            failure_threshold: 3,
            quality_failure_threshold: 5,
            success_threshold: 1,
            open_duration: Duration::from_secs(60),
        };
        let router =
            LlmRouter::new(Arc::new(MockLlm::new(100))).with_circuit_breaker_config(config);

        // First 3 calls fail normally (Inner errors)
        for _ in 0..3 {
            let result = router.complete(vec![], "").await;
            assert!(result.is_err());
        }

        // 4th call should be rejected immediately by open circuit
        let result = router.complete(vec![], "").await;
        assert!(result.is_err());
        let err_msg = result.unwrap_err().to_string();
        assert!(
            err_msg.contains("circuit breaker"),
            "expected circuit breaker error, got: {err_msg}"
        );
    }

    #[tokio::test]
    async fn fallback_provider_used_when_primary_cb_is_open() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            quality_failure_threshold: 5,
            success_threshold: 1,
            open_duration: Duration::from_secs(60),
        };
        // Primary fails to trip the CB, secondary always succeeds.
        let primary = Arc::new(MockLlm::new(100)); // always fails
        let secondary = Arc::new(MockLlm::new(0)); // always succeeds

        let router = LlmRouter::new(primary)
            .with_circuit_breaker_config(config)
            .add_provider("secondary", secondary, vec!["secondary/".into()])
            .with_fallback_providers(vec!["secondary".into()]);

        // Trip the default breaker (primary)
        for _ in 0..2 {
            let _ = router.complete(vec![], "").await;
        }

        // Primary CB is open; fallback should be used.
        // `complete_with_options` with no model → uses primary + fallback chain.
        let opts = CompletionOptions::default();
        let result = router.complete_with_options(vec![], "", &opts).await;
        assert!(result.is_ok(), "fallback should succeed when primary CB is open");
    }

    #[tokio::test]
    async fn all_providers_open_returns_error() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            quality_failure_threshold: 5,
            success_threshold: 1,
            open_duration: Duration::from_secs(60),
        };
        let primary = Arc::new(MockLlm::new(100));
        let secondary = Arc::new(MockLlm::new(100));

        let router = LlmRouter::new(primary.clone())
            .with_circuit_breaker_config(config.clone())
            .add_provider(
                "secondary",
                secondary.clone(),
                vec!["secondary/".into()],
            )
            .with_fallback_providers(vec!["secondary".into()]);

        // Trip primary CB
        for _ in 0..2 {
            let _ = router.complete(vec![], "").await;
        }
        // Trip secondary CB by calling it directly with options
        let opts = CompletionOptions {
            model: Some("secondary/model".into()),
            ..Default::default()
        };
        for _ in 0..2 {
            let _ = router.complete_with_options(vec![], "", &opts).await;
        }

        // Both CBs open: error with descriptive message
        let default_opts = CompletionOptions::default();
        let result = router.complete_with_options(vec![], "", &default_opts).await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn circuit_breaker_recovers() {
        let config = CircuitBreakerConfig {
            failure_threshold: 2,
            quality_failure_threshold: 5,
            success_threshold: 1,
            open_duration: Duration::from_millis(50),
        };
        // Will fail 2 times then succeed
        let router = LlmRouter::new(Arc::new(MockLlm::new(2))).with_circuit_breaker_config(config);

        // Trip the breaker
        for _ in 0..2 {
            let _ = router.complete(vec![], "").await;
        }

        // Wait for open_duration
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Should recover via half-open probe
        let result = router.complete(vec![], "").await;
        assert!(result.is_ok());
    }
}
