//! `orka-chart` — Chart generation skills for Orka agents.
//!
//! Provides a `create_chart` skill that renders bar, line, pie, scatter,
//! histogram, area, stacked-bar, and combo charts to in-memory PNG using
//! [`plotters`]. The PNG bytes are attached inline via [`MediaPayload`] and
//! delivered through every adapter layer without any external file storage.
//!
//! # Quick start
//!
//! ```no_run
//! use orka_chart::create_chart_skills;
//!
//! let skills = create_chart_skills();
//! // register each skill into your SkillRegistry
//! ```

pub mod config;
pub mod render;
pub mod skill;
pub mod types;

use std::sync::Arc;

pub use config::ChartConfig;
use orka_core::traits::Skill;
use thiserror::Error;

/// Errors produced by the chart rendering pipeline.
#[derive(Debug, Error)]
pub enum Error {
    /// Plotters drawing error.
    #[error("plotters error: {0}")]
    Plotters(String),

    /// Rendering logic error (bad data, empty series, etc.).
    #[error("render error: {0}")]
    Render(String),
}

impl From<Error> for orka_core::Error {
    fn from(e: Error) -> Self {
        orka_core::Error::Skill(e.to_string())
    }
}

/// Build all chart skills.
///
/// Returns a `Vec<Arc<dyn Skill>>` ready to be registered in the
/// `SkillRegistry`.
pub fn create_chart_skills() -> Vec<Arc<dyn Skill>> {
    vec![Arc::new(skill::ChartCreateSkill)]
}
