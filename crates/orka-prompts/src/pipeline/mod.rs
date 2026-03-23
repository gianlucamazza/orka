//! Configurable pipeline for building system prompts.
//!
//! This module provides:
//! - `SystemPromptPipeline`: Orchestrates prompt construction
//! - `PromptSection`: Trait for individual prompt sections
//! - `PipelineConfig`: Configuration for section ordering and separators

mod builder;
mod config;
mod section;

pub use builder::{BuildContext, SystemPromptPipeline};
pub use config::PipelineConfig;
pub use section::{DynamicSection, PromptSection, StaticSection};
