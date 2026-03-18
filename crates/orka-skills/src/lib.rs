//! Skill registry and plugin loading for agent tool use.
//!
//! - [`SkillRegistry`] — thread-safe registry of named [`Skill`](orka_core::traits::Skill) implementations
//! - [`WasmPluginSkill`] — WASM-based plugin skill loaded at runtime
//! - [`EchoSkill`] — built-in echo skill for testing

#![warn(missing_docs)]

/// Built-in skill implementations (e.g. [`EchoSkill`]).
pub mod builtins;
/// WASM plugin loader — scans a directory for `.wasm` skill files.
pub mod loader;
/// In-process skill registry keyed by skill name.
pub mod registry;
/// WASM-backed skill that executes a compiled plugin module via Wasmtime.
pub mod wasm_plugin;

pub use builtins::EchoSkill;
pub use loader::load_plugins;
pub use registry::SkillRegistry;
pub use wasm_plugin::WasmPluginSkill;

/// Create an empty [`SkillRegistry`].
pub fn create_skill_registry() -> SkillRegistry {
    SkillRegistry::new()
}
