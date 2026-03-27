use std::{path::Path, sync::Arc, time::Instant};

use orka_core::SkillInput;
use orka_skills::SkillRegistry;

use crate::{
    assertion::check_all,
    error::EvalResult,
    llm_judge::LlmJudge,
    report::{EvalReport, ScenarioResult},
    scenario::EvalFile,
};

/// Runs evaluation scenarios against a skill registry.
pub struct EvalRunner {
    registry: Arc<SkillRegistry>,
    judge: Option<LlmJudge>,
}

impl EvalRunner {
    /// Create a new runner backed by the given skill registry.
    pub fn new(registry: Arc<SkillRegistry>) -> Self {
        Self {
            registry,
            judge: None,
        }
    }

    /// Attach an LLM judge to evaluate semantic quality criteria.
    #[must_use]
    pub fn with_judge(mut self, judge: LlmJudge) -> Self {
        self.judge = Some(judge);
        self
    }

    /// Scan `dir` for `*.eval.toml` files and run all matching scenarios.
    ///
    /// If `skill_filter` is `Some`, only scenarios for that skill are run.
    pub async fn run_dir(
        &self,
        dir: &Path,
        skill_filter: Option<&str>,
    ) -> EvalResult<EvalReport> {
        let mut results = Vec::new();
        let start = Instant::now();

        let pattern = dir.join("*.eval.toml");
        let pattern_str = pattern.to_string_lossy();

        for path in glob::glob(&pattern_str)?.flatten() {
            let file_results = self.run_file(&path, skill_filter).await?;
            results.extend(file_results);
        }

        let total = results.len();
        let passed = results.iter().filter(|r| r.passed).count();
        let failed = total - passed;

        Ok(EvalReport {
            results,
            total,
            passed,
            failed,
            duration: start.elapsed(),
        })
    }

    /// Run all scenarios from a single eval file.
    async fn run_file(
        &self,
        path: &Path,
        skill_filter: Option<&str>,
    ) -> EvalResult<Vec<ScenarioResult>> {
        let content = std::fs::read_to_string(path)?;
        let eval: EvalFile = toml::from_str(&content)?;

        // Infer skill name from filename if not set
        let skill_name = eval.skill.clone().unwrap_or_else(|| {
            path.file_stem()
                .and_then(|s| s.to_str())
                .unwrap_or("unknown")
                .trim_end_matches(".eval")
                .to_string()
        });

        // Apply skill filter
        if skill_filter.is_some_and(|f| skill_name != f) {
            return Ok(Vec::new());
        }

        let mut results = Vec::new();

        for scenario in &eval.scenarios {
            let result = self.run_scenario(&skill_name, scenario).await;
            results.push(result);
        }

        Ok(results)
    }

    async fn run_scenario(
        &self,
        skill_name: &str,
        scenario: &crate::scenario::Scenario,
    ) -> ScenarioResult {
        let start = Instant::now();

        let input = SkillInput::new(scenario.input.clone());

        let invoke_result = self.registry.invoke(skill_name, input).await;
        let elapsed = start.elapsed();

        let mut assertions = check_all(&invoke_result, &scenario.expected, elapsed);

        // Run LLM judge if configured and there are judge criteria
        if let (Some(judge), Some(criteria)) = (&self.judge, &scenario.expected.judge)
            && !criteria.is_empty()
        {
            let output_str = match &invoke_result {
                Ok(out) => out.data.to_string(),
                Err(e) => format!("error: {e}"),
            };
            let input_val =
                serde_json::to_value(&scenario.input).unwrap_or(serde_json::Value::Null);
            let judge_results = judge.evaluate(criteria, &output_str, &input_val).await;
            assertions.extend(judge_results);
        }

        let passed = assertions.iter().all(|a| a.passed);
        let error = invoke_result.as_ref().err().map(ToString::to_string);

        ScenarioResult {
            skill: skill_name.to_string(),
            scenario: scenario.name.clone(),
            passed,
            assertions,
            duration: elapsed,
            error,
        }
    }
}
