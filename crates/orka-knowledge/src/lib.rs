//! Knowledge base and RAG (Retrieval-Augmented Generation) infrastructure.
//!
//! - [`EmbeddingProvider`] — trait for text-to-vector embedding
//! - [`VectorStore`] — trait for vector similarity search
//! - Skills: `memory_store`, `memory_search`, `doc_ingest`, `doc_list`

#![warn(missing_docs)]

/// Text chunking utilities for splitting documents into overlapping segments.
pub mod chunking;
/// Embedding providers: local ONNX (`local`) and OpenAI-compatible REST (`openai`).
pub mod embeddings;
/// Document parsers for HTML, Markdown, PDF, and plain text.
pub mod parsers;
/// Knowledge skills: `doc_ingest`, `doc_list`, `memory_search`, `memory_store`.
pub mod skills;
/// Core domain types: [`types::Chunk`], [`types::Document`], [`types::SearchResult`].
pub mod types;
/// Vector store trait and Qdrant backend implementation.
pub mod vector_store;

use std::sync::Arc;

use orka_core::Result;
use orka_core::config::KnowledgeConfig;
use orka_core::traits::Skill;
use tracing::info;

pub use embeddings::EmbeddingProvider;
pub use vector_store::VectorStore;

/// Create knowledge/RAG skills from config.
pub fn create_knowledge_skills(config: &KnowledgeConfig) -> Result<Vec<Arc<dyn Skill>>> {
    // Initialize embedding provider
    let embedding_provider: Arc<dyn EmbeddingProvider> = match config.embeddings.provider.as_str() {
        "openai" => {
            let api_key = std::env::var("OPENAI_API_KEY").map_err(|_| {
                orka_core::Error::Config(
                    "OPENAI_API_KEY required for openai embedding provider".into(),
                )
            })?;
            Arc::new(embeddings::openai::OpenAiEmbeddingProvider::new(
                api_key,
                config.embeddings.model.clone(),
                config.embeddings.dimensions,
            ))
        }
        _ => {
            // Default: local fastembed
            Arc::new(embeddings::local::LocalEmbeddingProvider::new(
                &config.embeddings.model,
                config.embeddings.dimensions,
            )?)
        }
    };

    // Initialize vector store
    let vector_store: Arc<dyn VectorStore> = Arc::new(vector_store::qdrant::QdrantStore::new(
        &config.vector_store.url,
    )?);

    create_knowledge_skills_with(config, embedding_provider, vector_store)
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
    let full_collection = format!(
        "{}{}",
        config.vector_store.collection_prefix, config.vector_store.default_collection
    );

    let memory_store: Arc<dyn Skill> = Arc::new(skills::memory_store::MemoryStoreSkill::new(
        embedding_provider.clone(),
        vector_store.clone(),
        full_collection.clone(),
    ));

    let memory_search: Arc<dyn Skill> = Arc::new(skills::memory_search::MemorySearchSkill::new(
        embedding_provider.clone(),
        vector_store.clone(),
        full_collection.clone(),
    ));

    let doc_ingest: Arc<dyn Skill> = Arc::new(skills::doc_ingest::DocIngestSkill::new(
        embedding_provider,
        vector_store.clone(),
        full_collection.clone(),
        config.chunking.chunk_size,
        config.chunking.chunk_overlap,
    ));

    let doc_list: Arc<dyn Skill> = Arc::new(skills::doc_list::DocListSkill::new(
        vector_store,
        full_collection,
    ));

    info!("knowledge skills initialized (memory_store, memory_search, doc_ingest, doc_list)");

    Ok(vec![memory_store, memory_search, doc_ingest, doc_list])
}
