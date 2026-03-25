//! Self-learning experience configuration.

use serde::Deserialize;

use crate::config::defaults;

/// Experience & self-learning configuration.
#[derive(Debug, Clone, Deserialize)]
#[non_exhaustive]
pub struct ExperienceConfig {
    /// Whether the experience / self-learning subsystem is enabled.
    #[serde(default = "defaults::default_experience_enabled")]
    pub enabled: bool,
    /// Maximum number of principles to inject into the system prompt.
    #[serde(default = "default_experience_max_principles")]
    pub max_principles: usize,
    /// Minimum relevance score (0.0–1.0) for a principle to be injected.
    #[serde(default = "default_experience_min_relevance")]
    pub min_relevance_score: f32,
    /// When to trigger reflection: "failures", "all", or "sampled".
    #[serde(default = "default_experience_reflect_on")]
    pub reflect_on: String,
    /// Sampling rate for reflection when `reflect_on` = "sampled" (0.0–1.0).
    #[serde(default = "default_experience_sample_rate")]
    pub sample_rate: f64,
    /// Qdrant collection name for principles.
    #[serde(default = "default_experience_principles_collection")]
    pub principles_collection: String,
    /// Qdrant collection name for raw trajectories.
    #[serde(default = "default_experience_trajectories_collection")]
    pub trajectories_collection: String,
    /// LLM model override for reflection calls (uses default if unset).
    #[serde(default)]
    pub reflection_model: Option<String>,
    /// Maximum tokens for the reflection LLM call.
    #[serde(default = "default_experience_reflection_max_tokens")]
    pub reflection_max_tokens: u32,
    /// Number of trajectories to load per offline distillation run.
    #[serde(default = "default_experience_distillation_batch_size")]
    pub distillation_batch_size: usize,
    /// Similarity threshold for principle deduplication (0.0–1.0).
    #[serde(default = "default_experience_dedup_threshold")]
    pub dedup_threshold: f32,
    /// How often to run offline distillation, in seconds (0 = disabled).
    #[serde(default = "defaults::default_experience_distillation_interval_secs")]
    pub distillation_interval_secs: u64,
}

impl ExperienceConfig {
    /// Validate the experience configuration.
    pub fn validate(&self) -> crate::Result<()> {
        if !(0.0..=1.0).contains(&self.min_relevance_score) {
            return Err(crate::Error::Config(format!(
                "experience.min_relevance must be in 0.0..=1.0, got: {}",
                self.min_relevance_score
            )));
        }
        if !(0.0..=1.0).contains(&self.sample_rate) {
            return Err(crate::Error::Config(format!(
                "experience.sample_rate must be in 0.0..=1.0, got: {}",
                self.sample_rate
            )));
        }
        Ok(())
    }
}

impl Default for ExperienceConfig {
    fn default() -> Self {
        Self {
            enabled: defaults::default_experience_enabled(),
            max_principles: default_experience_max_principles(),
            min_relevance_score: default_experience_min_relevance(),
            reflect_on: default_experience_reflect_on(),
            sample_rate: default_experience_sample_rate(),
            principles_collection: default_experience_principles_collection(),
            trajectories_collection: default_experience_trajectories_collection(),
            reflection_model: None,
            reflection_max_tokens: default_experience_reflection_max_tokens(),
            distillation_batch_size: default_experience_distillation_batch_size(),
            dedup_threshold: default_experience_dedup_threshold(),
            distillation_interval_secs: defaults::default_experience_distillation_interval_secs(),
        }
    }
}

fn default_experience_max_principles() -> usize {
    5
}

fn default_experience_min_relevance() -> f32 {
    0.6
}

fn default_experience_reflect_on() -> String {
    "failures".into()
}

fn default_experience_sample_rate() -> f64 {
    0.1
}

fn default_experience_principles_collection() -> String {
    "orka_principles".into()
}

fn default_experience_trajectories_collection() -> String {
    "orka_trajectories".into()
}

fn default_experience_reflection_max_tokens() -> u32 {
    1024
}

fn default_experience_distillation_batch_size() -> usize {
    20
}

fn default_experience_dedup_threshold() -> f32 {
    0.85
}
