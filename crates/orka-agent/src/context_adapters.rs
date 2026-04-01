//! Adapters for connecting orka-prompts traits to orka service implementations.
//!
//! This module provides bridge implementations that adapt the concrete services
//! (`ExperienceService`, `SoftSkillRegistry`) to the trait interfaces expected
//! by the context provider system in orka-prompts.

use std::sync::Arc;

use async_trait::async_trait;
use orka_core::Result;

/// Adapter for `ExperienceService` from orka-experience to orka-prompts trait.
pub(crate) struct ExperienceServiceAdapter {
    inner: Arc<orka_experience::ExperienceService>,
}

impl ExperienceServiceAdapter {
    /// Create a new adapter wrapping the real experience service.
    pub(crate) fn new(inner: Arc<orka_experience::ExperienceService>) -> Self {
        Self { inner }
    }
}

#[async_trait]
impl orka_prompts::context::ExperienceService for ExperienceServiceAdapter {
    async fn retrieve_principles(
        &self,
        query: &str,
        workspace: &str,
    ) -> Result<Vec<orka_prompts::context::Principle>> {
        let principles = self
            .inner
            .retrieve_principles(query, workspace)
            .await
            .map_err(|e| orka_core::Error::experience(e, "failed to retrieve principles"))?;

        Ok(principles
            .into_iter()
            .map(|p| orka_prompts::context::Principle {
                text: p.text,
                kind: match p.kind {
                    orka_experience::types::PrincipleKind::Do => {
                        orka_prompts::context::PrincipleKind::Do
                    }
                    orka_experience::types::PrincipleKind::Avoid => {
                        orka_prompts::context::PrincipleKind::Avoid
                    }
                },
            })
            .collect())
    }
}

/// Adapter for `SoftSkillRegistry` from orka-skills to orka-prompts trait.
pub(crate) struct SoftSkillRegistryAdapter {
    inner: Arc<orka_skills::SoftSkillRegistry>,
}

impl SoftSkillRegistryAdapter {
    /// Create a new adapter wrapping the real soft skill registry.
    pub(crate) fn new(inner: Arc<orka_skills::SoftSkillRegistry>) -> Self {
        Self { inner }
    }
}

impl orka_prompts::context::SoftSkillRegistry for SoftSkillRegistryAdapter {
    fn build_prompt_section(&self, names: &[&str]) -> String {
        self.inner.build_prompt_section(names)
    }

    fn list(&self) -> Vec<&str> {
        self.inner.list()
    }

    fn filter_by_message(&self, message: &str) -> Vec<&str> {
        self.inner.filter_by_message(message)
    }
}

/// Helper function to get the soft skill selection mode.
pub(crate) fn get_soft_skill_selection_mode(
    registry: &orka_skills::SoftSkillRegistry,
) -> orka_prompts::context::SoftSkillSelectionMode {
    match registry.selection_mode {
        orka_skills::SoftSkillSelectionMode::Keyword => {
            orka_prompts::context::SoftSkillSelectionMode::Keyword
        }
        orka_skills::SoftSkillSelectionMode::All => {
            orka_prompts::context::SoftSkillSelectionMode::All
        }
    }
}
