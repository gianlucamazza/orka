//! Context providers for dynamic prompt data.
//!
//! This module provides traits and implementations for gathering
//! contextual data to inject into prompts.

mod concrete_providers;
mod provider;
mod types;

pub use concrete_providers::{
    ContextCoordinator, ExperienceContextProvider, ExperienceService, Principle,
    SectionsContextProvider, ShellContextProvider, SoftSkillRegistry, SoftSkillsContextProvider,
};
pub use orka_core::{PrincipleKind, SoftSkillSelectionMode};
pub use provider::{ContextProvider, PrinciplesProvider, WorkspaceProvider};
pub use types::{PrincipleContext, SessionContext, WorkspaceContext};

// BuildContext is unified and exported from pipeline module
pub use crate::pipeline::BuildContext;
