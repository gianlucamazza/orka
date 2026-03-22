//! Skill registry and plugin loading for agent tool use.
//!
//! - [`SkillRegistry`] — thread-safe registry of named [`Skill`](orka_core::traits::Skill) implementations
//! - [`WasmPluginSkill`] — WASM-based plugin skill loaded at runtime
//! - [`EchoSkill`] — built-in echo skill for testing
//! - [`SoftSkillRegistry`] — registry for instruction-based soft skills loaded from `SKILL.md` directories

#![warn(missing_docs)]

/// Built-in skill implementations (e.g. [`EchoSkill`]).
pub mod builtins;
/// WASM plugin loader — scans a directory for `.wasm` skill files.
pub mod loader;
/// In-process skill registry keyed by skill name.
pub mod registry;
/// Scanner for soft skill directories.
pub mod soft_loader;
/// Registry for soft skills.
pub mod soft_registry;
/// Instruction-based soft skills loaded from SKILL.md directories.
pub mod soft_skill;
/// WASM-backed skill that executes a compiled plugin module via Wasmtime.
pub mod wasm_plugin;

pub use builtins::EchoSkill;
pub use loader::load_plugins;
pub use registry::SkillRegistry;
pub use soft_loader::scan_soft_skills;
pub use soft_registry::{SoftSkillRegistry, SoftSkillSelectionMode};
pub use soft_skill::{SoftSkill, SoftSkillMeta, SoftSkillSummary};
pub use wasm_plugin::WasmPluginSkill;

/// Create an empty [`SkillRegistry`].
pub fn create_skill_registry() -> SkillRegistry {
    SkillRegistry::new()
}
