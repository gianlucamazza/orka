pub mod builtins;
pub mod loader;
pub mod registry;
pub mod wasm_plugin;

pub use builtins::EchoSkill;
pub use loader::load_plugins;
pub use registry::SkillRegistry;
pub use wasm_plugin::WasmPluginSkill;

pub fn create_skill_registry() -> SkillRegistry {
    SkillRegistry::new()
}
