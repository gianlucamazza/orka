//! LLM-driven onboarding wizard for Orka.
//!
//! Provides an [`OnboardSession`] that guides users through configuring
//! `orka.toml` via a conversational LLM interface using tool-use.

pub(crate) mod config_builder;
pub(crate) mod session;
pub(crate) mod system_prompt;
pub(crate) mod tools;

pub(crate) use config_builder::ConfigBuilder;
pub(crate) use session::{BootstrapProvider, OnboardIo, OnboardSession};
