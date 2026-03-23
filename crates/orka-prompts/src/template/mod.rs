//! Template engine and registry for prompt management.
//!
//! This module provides:
//! - `TemplateEngine`: Handlebars-based template rendering
//! - `TemplateRegistry`: File-based template loading with hot-reload

mod engine;
mod loader;
mod registry;

pub use engine::{TemplateEngine, TemplateError};
pub use loader::TemplateLoader;
pub use registry::{TemplateRegistry, TemplateSource};
