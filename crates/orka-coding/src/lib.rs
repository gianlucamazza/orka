//! Coding delegation skills for Orka.
//!
//! Provides [`CodingDelegateSkill`] — a routing entrypoint that dispatches
//! coding tasks to the configured backend (Claude Code, Codex, or `OpenCode`)
//! and normalizes the result into a common [`SkillOutput`].

pub mod config;
mod skill;
mod stream;

pub use config::{
    ApprovalPolicy, ClaudeCodeConfig, CodexConfig, CodingConfig, CodingProvider,
    CodingProvidersConfig, CodingSelectionPolicy, OpenCodeConfig, SandboxMode,
};
pub use skill::CodingDelegateSkill;
