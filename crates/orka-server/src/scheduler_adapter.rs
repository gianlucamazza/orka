//! Bridge between `orka_skills::SkillRegistry` and
//! `orka_scheduler::SkillRegistry`.

use std::sync::Arc;

/// Adapter to bridge `orka_skills::SkillRegistry` with
/// `orka_scheduler::SkillRegistry` trait.
pub(crate) struct SchedulerSkillRegistryAdapter(pub Arc<orka_skills::SkillRegistry>);

#[async_trait::async_trait]
impl orka_scheduler::SkillRegistry for SchedulerSkillRegistryAdapter {
    async fn invoke(
        &self,
        name: &str,
        input: orka_core::SkillInput,
    ) -> orka_core::Result<orka_core::SkillOutput> {
        self.0.invoke(name, input).await
    }
}
