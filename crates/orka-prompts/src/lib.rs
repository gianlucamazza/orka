//! Template-based prompt management system for Orka.
//!
//! This crate provides a complete template engine for building system prompts,
//! with support for hot-reload, configurable pipelines, and context providers.
//!
//! # Example
//!
//! ```rust
//! use orka_prompts::template::{TemplateEngine, TemplateRegistry};
//!
//! async fn example() {
//!     // Create a template engine
//!     let mut engine = TemplateEngine::new();
//!     engine.register_template("greeting", "Hello, {{name}}!").unwrap();
//!     
//!     // Render with context
//!     let context = serde_json::json!({ "name": "World" });
//!     let result = engine.render("greeting", &context).unwrap();
//!     assert_eq!(result, "Hello, World!");
//! }
//! ```
#![warn(missing_docs)]

/// Prompt-system configuration.
pub mod config;
/// Centralized default values and constants.
pub mod defaults;

/// Context providers for dynamic prompt data.
pub mod context;

/// Configurable pipeline for building system prompts.
pub mod pipeline;

/// Template engine and registry with hot-reload support.
pub mod template;

pub use config::PromptsConfig;
pub use context::{BuildContext, ContextProvider, SessionContext};
pub use defaults::*;
pub use pipeline::{PipelineConfig, PromptSection, SystemPromptPipeline};
pub use template::{TemplateEngine, TemplateRegistry};
