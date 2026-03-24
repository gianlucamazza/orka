use std::time::Duration;

use crate::assertion::AssertionResult;

/// Result of a single scenario execution.
#[derive(Debug)]
pub struct ScenarioResult {
    /// Skill name.
    pub skill: String,
    /// Scenario name.
    pub scenario: String,
    /// Whether all assertions passed.
    pub passed: bool,
    /// Individual assertion results.
    pub assertions: Vec<AssertionResult>,
    /// Execution duration.
    pub duration: Duration,
    /// Error message if the skill returned an error.
    pub error: Option<String>,
}

/// Aggregated report for a run.
#[derive(Debug)]
pub struct EvalReport {
    /// All scenario results.
    pub results: Vec<ScenarioResult>,
    /// Total number of scenarios run.
    pub total: usize,
    /// Number of passing scenarios.
    pub passed: usize,
    /// Number of failing scenarios.
    pub failed: usize,
    /// Total duration of the run.
    pub duration: Duration,
}

impl EvalReport {
    /// Print a human-readable report to stdout.
    pub fn print_pretty(&self) {
        // Group by skill
        let mut by_skill: std::collections::HashMap<&str, Vec<&ScenarioResult>> =
            std::collections::HashMap::new();
        for r in &self.results {
            by_skill.entry(r.skill.as_str()).or_default().push(r);
        }

        let mut skills: Vec<&str> = by_skill.keys().copied().collect();
        skills.sort();

        for skill in skills {
            println!("\n{skill}");
            for r in &by_skill[skill] {
                let status = if r.passed { "PASS" } else { "FAIL" };
                println!("  [{status}] {} ({}ms)", r.scenario, r.duration.as_millis());
                if !r.passed {
                    for a in &r.assertions {
                        if !a.passed {
                            let detail = a.detail.as_deref().unwrap_or("");
                            println!("    - {}: {detail}", a.check);
                        }
                    }
                }
            }
        }

        println!(
            "\nResults: {}/{} passed ({}ms total)",
            self.passed,
            self.total,
            self.duration.as_millis()
        );
    }

    /// Serialize the report to JSON.
    pub fn to_json(&self) -> String {
        // Simple manual JSON serialization to avoid adding serde derive to internal
        // types
        let results: Vec<serde_json::Value> = self
            .results
            .iter()
            .map(|r| {
                serde_json::json!({
                    "skill": r.skill,
                    "scenario": r.scenario,
                    "passed": r.passed,
                    "duration_ms": r.duration.as_millis(),
                    "error": r.error,
                    "assertions": r.assertions.iter().map(|a| serde_json::json!({
                        "check": a.check,
                        "passed": a.passed,
                        "detail": a.detail,
                    })).collect::<Vec<_>>(),
                })
            })
            .collect();

        serde_json::json!({
            "total": self.total,
            "passed": self.passed,
            "failed": self.failed,
            "duration_ms": self.duration.as_millis(),
            "results": results,
        })
        .to_string()
    }
}
