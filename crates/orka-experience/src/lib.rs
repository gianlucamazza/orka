//! Self-learning experience system that extracts reusable principles from agent interactions.
//!
//! - [`TrajectoryCollector`] — records tool-use trajectories during conversations
//! - [`TrajectoryStore`] — persists raw trajectories for offline processing
//! - [`ExperienceService`] — reflects on outcomes, distills patterns, and stores/retrieves principles
//! - [`PrincipleStore`] — vector-backed storage for learned principles

#![warn(missing_docs)]

/// Trajectory collection during handler invocations.
pub mod collector;
/// Offline distillation of cross-trajectory patterns into principles.
pub mod distiller;
/// Single-trajectory principle reflection using an LLM.
pub mod reflector;
/// High-level facade combining collection, reflection, retrieval, and distillation.
pub mod service;
/// Vector-backed principle storage and retrieval.
pub mod store;
/// Trajectory persistence for offline distillation.
pub mod trajectory_store;
/// Core types: Trajectory, Principle, SkillTrace, OutcomeSignal.
pub mod types;
pub(crate) mod utils;

use std::sync::Arc;

use orka_core::Result;
use orka_core::config::ExperienceConfig;
use orka_knowledge::embeddings::EmbeddingProvider;
use orka_knowledge::vector_store::VectorStore;
use orka_llm::client::LlmClient;
use tracing::info;

pub use collector::TrajectoryCollector;
pub use service::{ExperienceService, ReflectionResult};
pub use store::PrincipleStore;
pub use trajectory_store::TrajectoryStore;
pub use types::{OutcomeSignal, Principle, PrincipleKind, StructuralAction, Trajectory};

/// Create the experience service from config and shared infrastructure.
///
/// Returns `None` if experience is disabled.
pub fn create_experience_service(
    config: &ExperienceConfig,
    embeddings: Arc<dyn EmbeddingProvider>,
    vector_store: Arc<dyn VectorStore>,
    llm: Arc<dyn LlmClient>,
) -> Result<Option<Arc<ExperienceService>>> {
    if !config.enabled {
        info!("experience system disabled");
        return Ok(None);
    }

    let principle_store = Arc::new(PrincipleStore::new(
        Arc::clone(&embeddings),
        Arc::clone(&vector_store),
        config.principles_collection.clone(),
    ));

    let trajectory_store = Arc::new(TrajectoryStore::new(
        embeddings,
        vector_store,
        config.trajectories_collection.clone(),
    ));

    let service = Arc::new(ExperienceService::new(
        principle_store,
        trajectory_store,
        llm,
        config.clone(),
    ));

    info!(
        collection = %config.principles_collection,
        trajectories_collection = %config.trajectories_collection,
        reflect_on = %config.reflect_on,
        max_principles = config.max_principles,
        distillation_batch_size = config.distillation_batch_size,
        "experience system initialized"
    );

    Ok(Some(service))
}
