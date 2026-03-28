//! Native autonomous research campaigns for Orka.
//!
//! This crate provides:
//! - persistent campaign, run, and candidate state;
//! - execution services that reuse Orka skills (`coding_delegate`, git, shell);
//! - public skills that expose research-oriented operations to the runtime.

#![warn(missing_docs)]

/// Research subsystem configuration.
pub mod config;
/// Campaign execution service.
pub mod service;
/// Runtime-facing research skills.
pub mod skills;
/// Persistence backends for campaigns, runs, and candidates.
pub mod store;
/// Core research domain types.
pub mod types;
/// Internal utilities (metric extraction).
pub(crate) mod util;

use std::sync::Arc;

pub use config::ResearchConfig;
use orka_checkpoint::CheckpointStore;
use orka_core::Result;
use orka_scheduler::ScheduleStore;
pub use service::ResearchService;
pub use skills::{
    CandidateCompareSkill, ExperimentRunSkill, ResearchCampaignRunSkill, ResearchPromoteSkill,
};
pub use store::{InMemoryResearchStore, RedisResearchStore, ResearchStore};
pub use types::{
    CandidateStatus, ComparisonDirection, CreateResearchCampaign, EvaluationMetricConfig,
    EvaluationResult, PromotionRequestStatus, PromotionSubmission, ResearchCampaign,
    ResearchCandidate, ResearchPromotionRequest, ResearchRun, ResearchRunStatus,
};

/// Build a Redis-backed research store from the configured Redis URL.
pub fn create_research_store(redis_url: &str) -> Result<Arc<dyn ResearchStore>> {
    Ok(Arc::new(RedisResearchStore::new(redis_url)?))
}

/// Build the research skills that should be registered in the global skill
/// registry.
pub fn create_research_skills(
    service: Arc<ResearchService>,
) -> Vec<Arc<dyn orka_core::traits::Skill>> {
    vec![
        Arc::new(ExperimentRunSkill::new(service.clone())),
        Arc::new(CandidateCompareSkill),
        Arc::new(ResearchPromoteSkill::new(service.clone())),
        Arc::new(ResearchCampaignRunSkill::new(service)),
    ]
}

/// Convenience constructor for a research service.
pub fn create_research_service(
    store: Arc<dyn ResearchStore>,
    scheduler_store: Option<Arc<dyn ScheduleStore>>,
    checkpoint_store: Option<Arc<dyn CheckpointStore>>,
    config: ResearchConfig,
    secrets: Arc<dyn orka_core::traits::SecretManager>,
    event_sink: Option<Arc<dyn orka_core::traits::EventSink>>,
) -> Arc<ResearchService> {
    Arc::new(ResearchService::new(
        store,
        scheduler_store,
        checkpoint_store,
        config,
        secrets,
        event_sink,
    ))
}
