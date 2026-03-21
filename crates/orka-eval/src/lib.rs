//! Skill evaluation framework for Orka.
//!
//! Provides a lightweight framework for testing skill effectiveness using
//! TOML-based scenario files.

pub mod assertion;
pub mod report;
pub mod runner;
pub mod scenario;

pub use report::{EvalReport, ScenarioResult};
pub use runner::EvalRunner;
pub use scenario::{EvalFile, Expectations, Scenario};
