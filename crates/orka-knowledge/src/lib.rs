//! Knowledge base and RAG (Retrieval-Augmented Generation) infrastructure.
//!
//! - [`EmbeddingProvider`] — trait for text-to-vector embedding
//! - [`VectorStore`] — trait for vector similarity search
//! - Skills: `remember_fact`, `search_facts`, `list_facts`, `forget_fact`,
//!   `ingest_document`, `list_documents`

#![warn(missing_docs)]

/// Text chunking utilities for splitting documents into overlapping segments.
pub mod chunking;
/// Knowledge/RAG configuration types.
pub mod config;
/// Embedding providers: local ONNX (`local`) and OpenAI-compatible REST
/// (`openai`).
pub mod embeddings;
/// Semantic fact storage with explicit memory semantics.
pub mod fact_store;
/// Document parsers for HTML, Markdown, PDF, and plain text.
pub mod parsers;
/// Memory and knowledge skills.
pub mod skills;
/// Core domain types: [`types::Chunk`], [`types::Document`],
/// [`types::SearchResult`].
pub mod types;
/// Vector store trait and Qdrant backend implementation.
pub mod vector_store;

use std::sync::Arc;

pub use config::{
    ChunkingConfig, EmbeddingProvider as EmbeddingProviderKind, EmbeddingsConfig, KnowledgeConfig,
    RetrievalConfig, VectorStoreBackend, VectorStoreConfig, default_qdrant_url,
};
pub use embeddings::EmbeddingProvider;
pub use fact_store::{FactRecord, FactStore};
use orka_core::{Result, traits::Skill};
use tracing::info;
pub use vector_store::VectorStore;

/// Create knowledge/RAG skills from config.
pub fn create_knowledge_skills(config: &KnowledgeConfig) -> Result<Vec<Arc<dyn Skill>>> {
    let (embedding_provider, vector_store) = create_embedding_and_store(config)?;
    create_knowledge_skills_with(config, embedding_provider, vector_store)
}

/// Create a semantic fact store from config.
pub fn create_fact_store(config: &KnowledgeConfig) -> Result<Arc<FactStore>> {
    let (embedding_provider, vector_store) = create_embedding_and_store(config)?;
    Ok(Arc::new(FactStore::new(
        embedding_provider,
        vector_store,
        config.vector_store.collection_name.clone(),
    )))
}

fn create_embedding_and_store(
    config: &KnowledgeConfig,
) -> Result<(Arc<dyn EmbeddingProvider>, Arc<dyn VectorStore>)> {
    let embedding_provider: Arc<dyn embeddings::EmbeddingProvider> = match config
        .embeddings
        .provider
    {
        #[cfg(feature = "openai-embeddings")]
        EmbeddingProviderKind::Openai => {
            let api_key = std::env::var("OPENAI_API_KEY").or_else(|_| {
                config.embeddings.api_key.clone().ok_or_else(|| {
                    orka_core::Error::Config(
                        "OPENAI_API_KEY required for openai embedding provider".into(),
                    )
                })
            })?;
            Arc::new(embeddings::openai::OpenAiEmbeddingProvider::new(
                api_key,
                config.embeddings.model.clone(),
                embeddings::OPENAI_EMBEDDING_DIMS,
            ))
        }
        #[cfg(not(feature = "openai-embeddings"))]
        EmbeddingProviderKind::Openai => {
            return Err(orka_core::Error::Config(
                "openai embedding provider requires the `openai-embeddings` feature".into(),
            ));
        }
        #[cfg(feature = "local-embeddings")]
        EmbeddingProviderKind::Anthropic
        | EmbeddingProviderKind::Custom
        | EmbeddingProviderKind::Local => Arc::new(embeddings::local::LocalEmbeddingProvider::new(
            &config.embeddings.model,
            embeddings::LOCAL_EMBEDDING_DIMS,
        )?),
        #[cfg(not(feature = "local-embeddings"))]
        EmbeddingProviderKind::Anthropic
        | EmbeddingProviderKind::Custom
        | EmbeddingProviderKind::Local => {
            return Err(orka_core::Error::Config(
                "local embedding provider requires the `local-embeddings` feature".into(),
            ));
        }
    };

    #[cfg(feature = "qdrant")]
    let vector_store: Arc<dyn VectorStore> = Arc::new(vector_store::qdrant::QdrantStore::new(
        config
            .vector_store
            .url
            .as_deref()
            .unwrap_or(&default_qdrant_url()),
    )?);

    #[cfg(not(feature = "qdrant"))]
    let vector_store: Arc<dyn VectorStore> = {
        return Err(orka_core::Error::Config(
            "qdrant vector store requires the `qdrant` feature".into(),
        ));
    };

    Ok((embedding_provider, vector_store))
}

/// Create knowledge/RAG skills with pre-built provider and store.
///
/// This variant is useful for testing (pass in-memory implementations) or
/// when the caller wants to control the embedding/vector backends directly.
pub fn create_knowledge_skills_with(
    config: &KnowledgeConfig,
    embedding_provider: Arc<dyn EmbeddingProvider>,
    vector_store: Arc<dyn VectorStore>,
) -> Result<Vec<Arc<dyn Skill>>> {
    let full_collection = config.vector_store.collection_name.clone();
    let fact_store = Arc::new(FactStore::new(
        embedding_provider.clone(),
        vector_store.clone(),
        full_collection.clone(),
    ));

    let remember_fact: Arc<dyn Skill> = Arc::new(skills::memory_store::RememberFactSkill::new(
        fact_store.clone(),
    ));

    let search_facts: Arc<dyn Skill> = Arc::new(skills::memory_search::SearchFactsSkill::new(
        fact_store.clone(),
    ));

    let list_facts: Arc<dyn Skill> =
        Arc::new(skills::list_facts::ListFactsSkill::new(fact_store.clone()));

    let forget_fact: Arc<dyn Skill> =
        Arc::new(skills::forget_fact::ForgetFactSkill::new(fact_store));

    let doc_ingest: Arc<dyn Skill> = Arc::new(skills::doc_ingest::IngestDocumentSkill::new(
        embedding_provider,
        vector_store.clone(),
        full_collection.clone(),
        config.chunking.chunk_size,
        config.chunking.chunk_overlap,
    ));

    let doc_list: Arc<dyn Skill> = Arc::new(skills::doc_list::ListDocumentsSkill::new(
        vector_store,
        full_collection,
    ));

    info!(
        "knowledge skills initialized (remember_fact, search_facts, list_facts, forget_fact, ingest_document, list_documents)"
    );

    Ok(vec![
        remember_fact,
        search_facts,
        list_facts,
        forget_fact,
        doc_ingest,
        doc_list,
    ])
}
