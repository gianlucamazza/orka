//! Context providers for dynamic prompt data.
//!
//! This module provides traits and implementations for gathering
//! contextual data to inject into prompts.

mod concrete_providers;
mod provider;
mod types;

pub use concrete_providers::{
    ContextCoordinator, ExperienceContextProvider, ExperienceService, Principle,
    PrincipleKind, SectionsContextProvider, ShellContextProvider,
    SoftSkillRegistry, SoftSkillSelectionMode, SoftSkillsContextProvider,
};
pub use provider::{ContextProvider, PrinciplesProvider, WorkspaceProvider};
pub use types::{BuildContext, PrincipleContext, SessionContext, WorkspaceContext};
