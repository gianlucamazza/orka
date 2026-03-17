//! Skill registry and plugin loading for agent tool use.
//!
//! - [`SkillRegistry`] — thread-safe registry of named [`Skill`](orka_core::traits::Skill) implementations
//! - [`WasmPluginSkill`] — WASM-based plugin skill loaded at runtime
//! - [`EchoSkill`] — built-in echo skill for testing

#![warn(missing_docs)]

#[allow(missing_docs)]
pub mod builtins;
#[allow(missing_docs)]
pub mod loader;
#[allow(missing_docs)]
pub mod registry;
#[allow(missing_docs)]
pub mod wasm_plugin;

pub use builtins::EchoSkill;
pub use loader::load_plugins;
pub use registry::SkillRegistry;
pub use wasm_plugin::WasmPluginSkill;

/// Create an empty [`SkillRegistry`].
pub fn create_skill_registry() -> SkillRegistry {
    SkillRegistry::new()
}
