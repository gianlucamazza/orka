//! Experience / self-learning service initialization.

use std::sync::Arc;

use orka_config::OrkaConfig;
use orka_core::SecretStr;
use orka_experience::ExperienceService;
use orka_llm::LlmAuthKind;

/// Create the experience / self-learning service from config.
///
/// Reuses the knowledge config for embedding provider and vector store
/// settings. Returns `None` if experience is disabled or initialization fails.
#[allow(clippy::too_many_lines)]
pub(crate) fn create_experience_service(
    config: &OrkaConfig,
) -> anyhow::Result<Option<Arc<ExperienceService>>> {
    use orka_knowledge::{embeddings::EmbeddingProvider, vector_store::VectorStore};

    let first_provider = config
        .llm
        .providers
        .first()
        .ok_or_else(|| anyhow::anyhow!("experience requires at least one LLM provider"))?;

    let auth_kind = first_provider.auth_kind;
    let credential = match auth_kind {
        LlmAuthKind::AuthToken | LlmAuthKind::Subscription => first_provider
            .auth_token
            .clone()
            .or_else(|| {
                first_provider
                    .auth_token_env
                    .as_ref()
                    .and_then(|env| std::env::var(env).ok())
            })
            .or_else(|| {
                std::env::var(crate::providers::default_auth_token_env_var(
                    &first_provider.provider,
                ))
                .ok()
            })
            .or_else(|| first_provider.api_key.clone())
            .or_else(|| {
                first_provider
                    .api_key_env
                    .as_ref()
                    .and_then(|env| std::env::var(env).ok())
            })
            .or_else(|| {
                std::env::var(crate::providers::default_env_var(&first_provider.provider)).ok()
            }),
        _ => first_provider
            .api_key
            .clone()
            .or_else(|| {
                first_provider
                    .api_key_env
                    .as_ref()
                    .and_then(|env| std::env::var(env).ok())
            })
            .or_else(|| {
                std::env::var(crate::providers::default_env_var(&first_provider.provider)).ok()
            }),
    }
    .ok_or_else(|| {
        anyhow::anyhow!(
            "experience reflection requires a credential for provider '{}'",
            first_provider.name
        )
    })?;
    let credential = SecretStr::new(credential);

    let model = config
        .experience
        .reflection_model
        .clone()
        .or_else(|| first_provider.model.clone())
        .unwrap_or_else(|| config.llm.default_model.clone());

    let reflection_llm: Arc<dyn orka_llm::LlmClient> = match first_provider.provider.as_str() {
        "openai" | "moonshot" => Arc::new(orka_llm::OpenAiClient::with_options(
            credential,
            model,
            first_provider.timeout_secs.unwrap_or(30),
            first_provider
                .max_tokens
                .unwrap_or(config.llm.default_max_tokens),
            first_provider.max_retries.unwrap_or(2),
            first_provider.base_url.clone().unwrap_or_else(|| {
                crate::providers::default_base_url(&first_provider.provider)
                    .unwrap_or(crate::providers::OPENAI_BASE_URL)
                    .to_string()
            }),
        )),
        "ollama" => Arc::new(orka_llm::OllamaClient::new(model)),
        _ => Arc::new(orka_llm::AnthropicClient::with_auth_options(
            credential,
            match auth_kind {
                LlmAuthKind::AuthToken | LlmAuthKind::Subscription => {
                    orka_llm::AnthropicAuthKind::Bearer
                }
                _ => orka_llm::AnthropicAuthKind::ApiKey,
            },
            model,
            first_provider.timeout_secs.unwrap_or(30),
            first_provider
                .max_tokens
                .unwrap_or(config.llm.default_max_tokens),
            first_provider.max_retries.unwrap_or(2),
            orka_llm::ANTHROPIC_API_VERSION.into(),
            first_provider.base_url.clone(),
        )),
    };

    // Create embedding provider (reusing knowledge config)
    let embedding_provider: Arc<dyn EmbeddingProvider> = match config.knowledge.embeddings.provider
    {
        orka_knowledge::EmbeddingProviderKind::Openai => {
            let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
                anyhow::anyhow!("OPENAI_API_KEY required for openai embedding provider")
            })?;
            Arc::new(
                orka_knowledge::embeddings::openai::OpenAiEmbeddingProvider::new(
                    SecretStr::new(api_key),
                    config.knowledge.embeddings.model.clone(),
                    orka_knowledge::embeddings::OPENAI_EMBEDDING_DIMS,
                ),
            )
        }
        _ => {
            #[cfg(feature = "local-embeddings")]
            {
                Arc::new(
                    orka_knowledge::embeddings::local::LocalEmbeddingProvider::new(
                        &config.knowledge.embeddings.model,
                        config
                            .knowledge
                            .vector_store
                            .dimension
                            .try_into()
                            .unwrap_or(orka_knowledge::embeddings::LOCAL_EMBEDDING_DIMS),
                    )
                    .map_err(|e| {
                        anyhow::anyhow!("failed to create local embedding provider: {e}")
                    })?,
                )
            }
            #[cfg(not(feature = "local-embeddings"))]
            {
                return Err(anyhow::anyhow!(
                    "local embedding provider not available in this build; \
                     set embeddings.provider = \"openai\" in orka.toml"
                ));
            }
        }
    };

    // Create vector store
    let vector_store: Arc<dyn VectorStore> =
        Arc::new(orka_knowledge::vector_store::qdrant::QdrantStore::new(
            config
                .knowledge
                .vector_store
                .url
                .as_deref()
                .unwrap_or(&orka_knowledge::default_qdrant_url()),
        ));

    let service = orka_experience::create_experience_service(
        &config.experience,
        embedding_provider,
        vector_store,
        reflection_llm,
    )
    .map_err(|e| anyhow::anyhow!("failed to create experience service: {e}"))?;

    Ok(service)
}
