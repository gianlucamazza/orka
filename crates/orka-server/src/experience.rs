//! Experience / self-learning service initialization.

use std::sync::Arc;

use orka_core::config::OrkaConfig;
use orka_experience::ExperienceService;

/// Create the experience / self-learning service from config.
///
/// Reuses the knowledge config for embedding provider and vector store
/// settings. Returns `None` if experience is disabled or initialization fails.
pub(crate) fn create_experience_service(
    config: &OrkaConfig,
) -> anyhow::Result<Option<Arc<ExperienceService>>> {
    use orka_knowledge::{embeddings::EmbeddingProvider, vector_store::VectorStore};

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
        .or_else(|| std::env::var(crate::providers::default_env_var(&first_provider.provider)).ok())
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
        .or_else(|| first_provider.model.clone())
        .unwrap_or_else(|| config.llm.default_model.clone());

    let reflection_llm: Arc<dyn orka_llm::LlmClient> = match first_provider.provider.as_str() {
        "openai" => Arc::new(orka_llm::OpenAiClient::new(api_key, model)),
        "ollama" => Arc::new(orka_llm::OllamaClient::new(model)),
        _ => Arc::new(orka_llm::AnthropicClient::with_options(
            api_key,
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
        orka_core::config::primitives::EmbeddingProvider::Openai => {
            let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
                anyhow::anyhow!("OPENAI_API_KEY required for openai embedding provider")
            })?;
            Arc::new(
                orka_knowledge::embeddings::openai::OpenAiEmbeddingProvider::new(
                    api_key,
                    config.knowledge.embeddings.model.clone(),
                    orka_knowledge::embeddings::OPENAI_EMBEDDING_DIMS,
                ),
            )
        }
        _ => Arc::new(
            orka_knowledge::embeddings::local::LocalEmbeddingProvider::new(
                &config.knowledge.embeddings.model,
                config
                    .knowledge
                    .vector_store
                    .dimension
                    .try_into()
                    .unwrap_or(orka_knowledge::embeddings::LOCAL_EMBEDDING_DIMS),
            )
            .map_err(|e| anyhow::anyhow!("failed to create local embedding provider: {e}"))?,
        ),
    };

    // Create vector store
    let vector_store: Arc<dyn VectorStore> = Arc::new(
        orka_knowledge::vector_store::qdrant::QdrantStore::new(
            config
                .knowledge
                .vector_store
                .url
                .as_deref()
                .unwrap_or(&orka_core::config::defaults::default_qdrant_url()),
        )
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
