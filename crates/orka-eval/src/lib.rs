//! Skill evaluation framework for Orka.
//!
//! Provides a lightweight framework for testing skill effectiveness using
//! TOML-based scenario files.

/// Assertion and expectation evaluation primitives.
pub mod assertion;
/// Error types for the evaluation framework.
pub mod error;
/// LLM-as-a-judge helpers and abstractions.
pub mod llm_judge;
/// Report types emitted by evaluation runs.
pub mod report;
/// Evaluation runner implementation.
pub mod runner;
/// Scenario file schema and parsing types.
pub mod scenario;

pub use error::{EvalError, EvalResult};
pub use llm_judge::LlmJudge;
pub use report::{EvalReport, ScenarioResult};
pub use runner::EvalRunner;
pub use scenario::{EvalFile, Expectations, JudgeCriterion, Scenario};
