use std::time::Duration;

use orka_core::SkillOutput;

use crate::scenario::Expectations;

/// Result of a single assertion check.
#[derive(Debug, Clone)]
pub struct AssertionResult {
    /// Human-readable description of the check.
    pub check: String,
    /// Whether the check passed.
    pub passed: bool,
    /// Optional detail message on failure.
    pub detail: Option<String>,
}

impl AssertionResult {
    fn pass(check: impl Into<String>) -> Self {
        Self {
            check: check.into(),
            passed: true,
            detail: None,
        }
    }

    fn fail(check: impl Into<String>, detail: impl Into<String>) -> Self {
        Self {
            check: check.into(),
            passed: false,
            detail: Some(detail.into()),
        }
    }
    
    /// Create a passing assertion result (public API for tests).
    pub fn pass_test(check: impl Into<String>) -> Self {
        Self::pass(check)
    }
    
    /// Create a failing assertion result (public API for tests).
    pub fn fail_test(check: impl Into<String>, detail: impl Into<String>) -> Self {
        Self::fail(check, detail)
    }
}

/// Run all assertion checks for a scenario result.
pub fn check_all(
    result: &std::result::Result<SkillOutput, orka_core::Error>,
    expected: &Expectations,
    elapsed: Duration,
) -> Vec<AssertionResult> {
    let mut checks = Vec::new();

    // is_ok / is_error check
    if let Some(expect_ok) = expected.is_ok {
        match (expect_ok, result.is_ok()) {
            (true, true) | (false, false) => {
                checks.push(AssertionResult::pass("is_ok"));
            }
            (true, false) => {
                let err = result.as_ref().unwrap_err().to_string();
                checks.push(AssertionResult::fail(
                    "is_ok",
                    format!("expected Ok, got Err: {err}"),
                ));
            }
            (false, true) => {
                checks.push(AssertionResult::fail("is_ok", "expected Err, got Ok"));
            }
        }
    }

    // Content checks only make sense if we have output
    if let Ok(output) = result {
        let output_str = output.data.to_string();

        if let Some(contains) = &expected.contains {
            for needle in contains {
                if output_str.contains(needle.as_str()) {
                    checks.push(AssertionResult::pass(format!("contains {needle:?}")));
                } else {
                    checks.push(AssertionResult::fail(
                        format!("contains {needle:?}"),
                        format!("not found in: {}", truncate(&output_str, 200)),
                    ));
                }
            }
        }

        if let Some(not_contains) = &expected.not_contains {
            for needle in not_contains {
                if !output_str.contains(needle.as_str()) {
                    checks.push(AssertionResult::pass(format!("not_contains {needle:?}")));
                } else {
                    checks.push(AssertionResult::fail(
                        format!("not_contains {needle:?}"),
                        format!("found in: {}", truncate(&output_str, 200)),
                    ));
                }
            }
        }

        if let Some(fmt) = &expected.format {
            match fmt.as_str() {
                "json" => {
                    if serde_json::from_str::<serde_json::Value>(&output_str).is_ok() {
                        checks.push(AssertionResult::pass("format=json"));
                    } else {
                        checks.push(AssertionResult::fail(
                            "format=json",
                            "output is not valid JSON",
                        ));
                    }
                }
                other => {
                    checks.push(AssertionResult::fail(
                        format!("format={other}"),
                        format!("unknown format: {other}"),
                    ));
                }
            }
        }

        if let Some(pattern) = &expected.output_matches {
            match regex::Regex::new(pattern) {
                Ok(re) => {
                    if re.is_match(&output_str) {
                        checks.push(AssertionResult::pass(format!("output_matches {pattern:?}")));
                    } else {
                        checks.push(AssertionResult::fail(
                            format!("output_matches {pattern:?}"),
                            format!("no match in: {}", truncate(&output_str, 200)),
                        ));
                    }
                }
                Err(e) => {
                    checks.push(AssertionResult::fail(
                        format!("output_matches {pattern:?}"),
                        format!("invalid regex: {e}"),
                    ));
                }
            }
        }
    }

    // Duration check
    if let Some(max_ms) = expected.max_duration_ms {
        let actual_ms = elapsed.as_millis() as u64;
        if actual_ms <= max_ms {
            checks.push(AssertionResult::pass(format!("duration <= {max_ms}ms")));
        } else {
            checks.push(AssertionResult::fail(
                format!("duration <= {max_ms}ms"),
                format!("took {actual_ms}ms"),
            ));
        }
    }

    checks
}

fn truncate(s: &str, max: usize) -> &str {
    if s.len() <= max { s } else { &s[..max] }
}
