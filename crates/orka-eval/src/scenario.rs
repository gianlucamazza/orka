use std::collections::HashMap;

use serde::Deserialize;

/// Top-level structure of a `*.eval.toml` file.
#[derive(Debug, Deserialize)]
pub struct EvalFile {
    /// Skill name these scenarios target (inferred from filename if omitted).
    pub skill: Option<String>,
    /// List of test scenarios.
    pub scenarios: Vec<Scenario>,
}

/// A single evaluation scenario.
#[derive(Debug, Deserialize)]
pub struct Scenario {
    /// Unique name for this scenario.
    pub name: String,
    /// Human-readable description.
    pub description: Option<String>,
    /// Input arguments passed to the skill.
    pub input: HashMap<String, serde_json::Value>,
    /// Expected outcome checks.
    pub expected: Expectations,
}

/// Expected outcome for a scenario.
#[derive(Debug, Deserialize, Default)]
pub struct Expectations {
    /// The output JSON string must contain all these substrings.
    pub contains: Option<Vec<String>>,
    /// The output JSON string must NOT contain any of these substrings.
    pub not_contains: Option<Vec<String>>,
    /// If `"json"`, the output data must be valid JSON.
    pub format: Option<String>,
    /// If `true`, the skill must succeed. If `false`, it must fail.
    pub is_ok: Option<bool>,
    /// Regex pattern the output string must match.
    pub output_matches: Option<String>,
    /// Maximum allowed duration in milliseconds.
    pub max_duration_ms: Option<u64>,
    /// Semantic quality criteria evaluated by an LLM judge.
    ///
    /// Requires [`EvalRunner`] to be configured with a judge via
    /// [`EvalRunner::with_judge`]. Skipped silently when no judge is set.
    ///
    /// [`EvalRunner`]: crate::runner::EvalRunner
    pub judge: Option<Vec<JudgeCriterion>>,
}

/// A single semantic quality criterion for LLM-as-Judge evaluation.
#[derive(Debug, Deserialize, Clone)]
pub struct JudgeCriterion {
    /// Short identifier for this criterion (e.g. `"helpfulness"`).
    pub name: String,
    /// Human-readable description of what a passing response looks like.
    pub description: String,
    /// Optional rubric with explicit pass/fail guidance for the judge.
    pub rubric: Option<String>,
}
