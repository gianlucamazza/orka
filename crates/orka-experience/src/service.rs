use std::sync::Arc;

use std::fmt::Write as _;

use orka_core::{ErrorCategory, Result, config::ExperienceConfig};
use orka_llm::client::LlmClient;
use orka_prompts::template::TemplateRegistry;
use rand::Rng as _;
use tracing::{debug, info, warn};

use crate::{
    collector::TrajectoryCollector,
    distiller::Distiller,
    reflector::PrincipleReflector,
    store::PrincipleStore,
    trajectory_store::TrajectoryStore,
    types::{Principle, PrincipleKind, StructuralAction, Trajectory},
};

/// Result of a reflection pass: principles for prompt injection and structural
/// actions to apply.
pub struct ReflectionResult {
    /// Number of principles created or updated by this reflection.
    pub principles_created: usize,
    /// Structural actions derived deterministically from the trajectory.
    pub actions: Vec<StructuralAction>,
}

/// High-level facade combining trajectory collection, principle reflection,
/// retrieval, trajectory persistence, and offline distillation.
pub struct ExperienceService {
    store: Arc<PrincipleStore>,
    trajectory_store: Arc<TrajectoryStore>,
    reflector: PrincipleReflector,
    distiller: Distiller,
    config: ExperienceConfig,
    templates: Option<Arc<TemplateRegistry>>,
}

impl ExperienceService {
    /// Create a new experience service from its dependencies.
    pub fn new(
        store: Arc<PrincipleStore>,
        trajectory_store: Arc<TrajectoryStore>,
        llm: Arc<dyn LlmClient>,
        config: ExperienceConfig,
    ) -> Self {
        let reflector = PrincipleReflector::new(
            Arc::clone(&llm),
            config.reflection_model.clone(),
            config.reflection_max_tokens,
        );
        let distiller = Distiller::new(
            llm,
            config.reflection_model.clone(),
            config.reflection_max_tokens * 4, // distillation needs more tokens
        );
        Self {
            store,
            trajectory_store,
            reflector,
            distiller,
            config,
            templates: None,
        }
    }

    /// Set the template registry for prompt rendering.
    #[must_use]
    pub fn with_templates(mut self, templates: Arc<TemplateRegistry>) -> Self {
        self.templates = Some(Arc::clone(&templates));
        self.reflector = self.reflector.with_templates(templates.clone());
        self.distiller = self.distiller.with_templates(templates);
        self
    }

    /// Retrieve relevant principles for a user message in the given workspace.
    pub async fn retrieve_principles(
        &self,
        user_message: &str,
        workspace: &str,
    ) -> Result<Vec<Principle>> {
        if !self.config.enabled {
            return Ok(Vec::new());
        }

        // Search for principles matching both the workspace scope and "global"
        let mut principles = self
            .store
            .retrieve(
                user_message,
                self.config.max_principles * 2, // fetch extra to filter
                self.config.min_relevance_score,
                None, // no scope filter — we'll filter in code
            )
            .await?;

        // Keep only principles that match the workspace or are global
        principles.retain(|p| p.scope == workspace || p.scope == "global");

        // Truncate to max
        principles.truncate(self.config.max_principles);

        debug!(
            count = principles.len(),
            workspace, "retrieved principles for prompt enrichment"
        );
        Ok(principles)
    }

    /// List stored principles for a workspace, including global ones.
    pub async fn list_principles(&self, workspace: &str, limit: usize) -> Result<Vec<Principle>> {
        if !self.config.enabled {
            return Ok(Vec::new());
        }

        let mut principles = self.store.list(limit * 2, None).await?;
        principles.retain(|p| p.scope == workspace || p.scope == "global");
        principles.truncate(limit);
        Ok(principles)
    }

    /// Delete a stored principle by stable identifier.
    pub async fn forget_principle(&self, id: &str) -> Result<bool> {
        if !self.config.enabled {
            return Ok(false);
        }
        self.store.forget(id).await
    }

    /// Format principles for injection into the system prompt.
    ///
    /// This synchronous version uses the default formatting.
    /// For template-based formatting, use [`Self::format_principles`].
    pub fn format_principles_section(principles: &[Principle]) -> String {
        use orka_prompts::defaults::{
            PRINCIPLE_PREFIX_AVOID, PRINCIPLE_PREFIX_DO, PRINCIPLES_SECTION_HEADER,
            SECTION_SEPARATOR,
        };

        if principles.is_empty() {
            return String::new();
        }

        let mut section = String::from(SECTION_SEPARATOR);
        section.push_str(PRINCIPLES_SECTION_HEADER);
        section.push_str("\n\n");
        section.push_str(
            "The following principles were learned from past interactions. Apply them when relevant:\n\n",
        );

        for (i, p) in principles.iter().enumerate() {
            let prefix = match p.kind {
                PrincipleKind::Do => PRINCIPLE_PREFIX_DO,
                PrincipleKind::Avoid => PRINCIPLE_PREFIX_AVOID,
            };
            writeln!(section, "{}. [{}] {}", i + 1, prefix, p.text).unwrap_or(());
        }

        section
    }

    /// Format principles for injection using the configured templates.
    ///
    /// If templates are configured and a "principles" template exists, it will
    /// be used. Otherwise falls back to the default formatting.
    pub async fn format_principles(&self, principles: &[Principle]) -> String {
        if principles.is_empty() {
            return String::new();
        }

        // Try to use template if available
        if let Some(templates) = &self.templates {
            let context = serde_json::json!({
                "principles": principles.iter().map(|p| serde_json::json!({
                    "text": p.text,
                    "kind": match p.kind {
                        PrincipleKind::Do => "do",
                        PrincipleKind::Avoid => "avoid",
                    },
                    "scope": p.scope,
                })).collect::<Vec<_>>(),
            });

            match templates.render("principles", &context).await {
                Ok(rendered) => return rendered,
                Err(e) => {
                    tracing::debug!(error = %e, "failed to render principles template, using default");
                }
            }
        }

        // Fallback to default implementation
        Self::format_principles_section(principles)
    }

    /// Persist a trajectory to the trajectory store for future distillation.
    pub async fn record_trajectory(&self, trajectory: &Trajectory) -> Result<()> {
        if !self.config.enabled {
            return Ok(());
        }
        self.trajectory_store.store(trajectory).await
    }

    /// Decide whether to reflect on a trajectory and, if so, perform
    /// reflection.
    ///
    /// Returns a [`ReflectionResult`] with the number of principles created and
    /// any structural actions (e.g. skill disabling) derived
    /// deterministically from the trajectory.
    pub async fn maybe_reflect(&self, trajectory: &Trajectory) -> Result<ReflectionResult> {
        let actions = Self::derive_structural_actions(trajectory);

        if !self.config.enabled {
            return Ok(ReflectionResult {
                principles_created: 0,
                actions,
            });
        }

        if !self.should_reflect(trajectory) {
            return Ok(ReflectionResult {
                principles_created: 0,
                actions,
            });
        }

        let principles = self
            .reflector
            .reflect(trajectory, &trajectory.workspace)
            .await?;

        if principles.is_empty() {
            return Ok(ReflectionResult {
                principles_created: 0,
                actions,
            });
        }

        let stored = self
            .store
            .store_batch(&principles, self.config.dedup_threshold)
            .await?;

        info!(
            trajectory_id = %trajectory.id,
            principles_created = stored,
            "reflection completed"
        );

        Ok(ReflectionResult {
            principles_created: stored,
            actions,
        })
    }

    /// Derive structural actions deterministically from a trajectory (no LLM
    /// involved).
    fn derive_structural_actions(trajectory: &Trajectory) -> Vec<StructuralAction> {
        trajectory
            .skills_used
            .iter()
            .filter(|s| !s.success && s.error_category == Some(ErrorCategory::Environmental))
            .map(|s| StructuralAction::DisableSkill {
                skill_name: s.name.clone(),
                reason: s.error_message.clone().unwrap_or_default(),
            })
            .collect()
    }

    /// Run offline distillation over recent trajectories.
    ///
    /// Loads up to `distillation_batch_size` recent trajectories from the
    /// workspace, synthesizes cross-trajectory patterns, and stores the
    /// resulting principles.
    ///
    /// Returns the number of new principles created.
    pub async fn distill(&self, workspace: &str) -> Result<usize> {
        if !self.config.enabled {
            return Ok(0);
        }

        let trajectories = self
            .trajectory_store
            .load_recent(Some(workspace), self.config.distillation_batch_size)
            .await?;

        if trajectories.is_empty() {
            debug!(workspace, "no trajectories available for distillation");
            return Ok(0);
        }

        let principles = self.distiller.distill(&trajectories, workspace).await?;

        if principles.is_empty() {
            return Ok(0);
        }

        let created = self
            .store
            .store_batch(&principles, self.config.dedup_threshold)
            .await?;

        info!(
            workspace,
            trajectory_count = trajectories.len(),
            principles_created = created,
            "offline distillation completed"
        );

        Ok(created)
    }

    fn should_reflect(&self, trajectory: &Trajectory) -> bool {
        match self.config.reflect_on.as_str() {
            "all" => true,
            "failures" => !trajectory.success,
            "sampled" => {
                if trajectory.success {
                    let sample: f64 = rand::rng().random();
                    sample < self.config.sample_rate
                } else {
                    // Always reflect on failures
                    true
                }
            }
            other => {
                warn!(
                    reflect_on = other,
                    "unknown reflect_on value, defaulting to failures-only"
                );
                !trajectory.success
            }
        }
    }

    /// Create a trajectory collector for a new handler invocation.
    pub fn collector(
        &self,
        session_id: String,
        workspace: String,
        user_message: String,
    ) -> TrajectoryCollector {
        TrajectoryCollector::new(session_id, workspace, user_message)
    }

    /// Check if the experience system is enabled.
    pub fn is_enabled(&self) -> bool {
        self.config.enabled
    }
}
