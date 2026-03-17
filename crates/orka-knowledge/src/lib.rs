//! Knowledge base and RAG (Retrieval-Augmented Generation) infrastructure.
//!
//! - [`EmbeddingProvider`] — trait for text-to-vector embedding
//! - [`VectorStore`] — trait for vector similarity search
//! - Skills: `memory_store`, `memory_search`, `doc_ingest`, `doc_list`

#![warn(missing_docs)]

#[allow(missing_docs)]
pub mod chunking;
#[allow(missing_docs)]
pub mod embeddings;
#[allow(missing_docs)]
pub mod parsers;
#[allow(missing_docs)]
pub mod skills;
#[allow(missing_docs)]
pub mod types;
#[allow(missing_docs)]
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
