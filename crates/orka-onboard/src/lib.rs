//! LLM-driven onboarding wizard for Orka.
//!
//! Provides an [`OnboardSession`] that guides users through configuring
//! `orka.toml` via a conversational LLM interface using tool-use.
//!
//! # Architecture
//!
//! - [`ConfigBuilder`]: wraps `toml_edit::DocumentMut` with progressive
//!   mutation and round-trip validation via [`OrkaConfig`].
//! - [`OnboardIo`]: terminal I/O trait implemented by the CLI layer.
//! - [`OnboardSession`]: orchestrates the streaming tool-use loop.
//!
//! # Usage
//!
//! ```ignore
//! use orka_onboard::{OnboardSession, BootstrapProvider};
//!
//! let session = OnboardSession::new(llm_client, secrets, provider);
//! let toml = session.run(&mut my_io).await?;
//! std::fs::write("orka.toml", toml)?;
//! ```

#![warn(missing_docs)]

pub mod config_builder;
pub mod session;
pub mod system_prompt;
pub mod tools;

pub use config_builder::ConfigBuilder;
pub use session::{BootstrapProvider, OnboardIo, OnboardSession};
