pub mod scheduler;
pub mod skills;
pub mod store;
pub mod types;

use std::sync::Arc;

use orka_core::config::SchedulerConfig;
use orka_core::traits::Skill;
use orka_core::Result;
use tracing::info;

pub use scheduler::{Scheduler, SkillRegistry};
pub use store::RedisScheduleStore;

/// A list of skills bundled with the schedule store needed to run the scheduler loop.
pub type SchedulerSkills = (Vec<Arc<dyn Skill>>, Arc<RedisScheduleStore>);

/// Create scheduler skills and the schedule store.
///
/// Returns `(skills, store)` — the store is needed to start the scheduler loop.
pub fn create_scheduler_skills(
    _config: &SchedulerConfig,
    redis_url: &str,
) -> Result<SchedulerSkills> {
    let store = Arc::new(RedisScheduleStore::new(redis_url)?);

    let create: Arc<dyn Skill> = Arc::new(skills::schedule_create::ScheduleCreateSkill::new(
        store.clone(),
    ));
    let list: Arc<dyn Skill> =
        Arc::new(skills::schedule_list::ScheduleListSkill::new(store.clone()));
    let delete: Arc<dyn Skill> = Arc::new(skills::schedule_delete::ScheduleDeleteSkill::new(
        store.clone(),
    ));

    info!("scheduler skills initialized (schedule_create, schedule_list, schedule_delete)");

    Ok((vec![create, list, delete], store))
}
