//! Integration tests for orka-eval framework.

use std::sync::Arc;
use std::time::Duration;

use orka_core::{Error, SkillOutput};
use orka_eval::assertion::{check_all, AssertionResult};
use orka_eval::report::{EvalReport, ScenarioResult};
use orka_eval::scenario::{EvalFile, Expectations, Scenario};
use orka_skills::SkillRegistry;

// Helper to create SkillOutput for tests
fn make_output(data: serde_json::Value) -> SkillOutput {
    // Use serde_json to create the struct since it's non-exhaustive
    serde_json::from_value(serde_json::json!({
        "data": data
    })).unwrap()
}

#[test]
fn test_assertion_result_pass() {
    let result = AssertionResult::pass_test("test check");
    assert!(result.passed);
    assert_eq!(result.check, "test check");
    assert!(result.detail.is_none());
}

#[test]
fn test_assertion_result_fail() {
    let result = AssertionResult::fail_test("test check", "detail message");
    assert!(!result.passed);
    assert_eq!(result.check, "test check");
    assert_eq!(result.detail, Some("detail message".to_string()));
}

#[tokio::test]
async fn test_check_all_is_ok_true_success() {
    let output = make_output(serde_json::json!({"result": "success"}));
    let result: Result<SkillOutput, Error> = Ok(output);
    let expectations = Expectations {
        is_ok: Some(true),
        ..Default::default()
    };

    let checks = check_all(&result, &expectations, Duration::from_millis(10));
    assert_eq!(checks.len(), 1);
    assert!(checks[0].passed);
    assert_eq!(checks[0].check, "is_ok");
}

#[tokio::test]
async fn test_check_all_is_ok_true_failure() {
    let error = Error::Skill("test error".to_string());
    let result: Result<SkillOutput, Error> = Err(error);
    let expectations = Expectations {
        is_ok: Some(true),
        ..Default::default()
    };

    let checks = check_all(&result, &expectations, Duration::from_millis(10));
    assert_eq!(checks.len(), 1);
    assert!(!checks[0].passed);
    assert_eq!(checks[0].check, "is_ok");
    assert!(checks[0].detail.as_ref().unwrap().contains("expected Ok"));
}

#[tokio::test]
async fn test_check_all_is_ok_false_success() {
    let error = Error::Skill("test error".to_string());
    let result: Result<SkillOutput, Error> = Err(error);
    let expectations = Expectations {
        is_ok: Some(false),
        ..Default::default()
    };

    let checks = check_all(&result, &expectations, Duration::from_millis(10));
    assert_eq!(checks.len(), 1);
    assert!(checks[0].passed);
    assert_eq!(checks[0].check, "is_ok");
}

#[tokio::test]
async fn test_check_all_contains_success() {
    let output = make_output(serde_json::json!({"message": "hello world"}));
    let result: Result<SkillOutput, Error> = Ok(output);
    let expectations = Expectations {
        contains: Some(vec!["hello".to_string(), "world".to_string()]),
        ..Default::default()
    };

    let checks = check_all(&result, &expectations, Duration::from_millis(10));
    assert_eq!(checks.len(), 2);
    assert!(checks.iter().all(|c| c.passed));
}

#[tokio::test]
async fn test_check_all_contains_failure() {
    let output = make_output(serde_json::json!({"message": "hello"}));
    let result: Result<SkillOutput, Error> = Ok(output);
    let expectations = Expectations {
        contains: Some(vec!["notfound".to_string()]),
        ..Default::default()
    };

    let checks = check_all(&result, &expectations, Duration::from_millis(10));
    assert_eq!(checks.len(), 1);
    assert!(!checks[0].passed);
    assert_eq!(checks[0].check, "contains \"notfound\"");
}

#[tokio::test]
async fn test_check_all_not_contains_success() {
    let output = make_output(serde_json::json!({"message": "hello"}));
    let result: Result<SkillOutput, Error> = Ok(output);
    let expectations = Expectations {
        not_contains: Some(vec!["goodbye".to_string()]),
        ..Default::default()
    };

    let checks = check_all(&result, &expectations, Duration::from_millis(10));
    assert_eq!(checks.len(), 1);
    assert!(checks[0].passed);
}

#[tokio::test]
async fn test_check_all_not_contains_failure() {
    let output = make_output(serde_json::json!({"message": "hello world"}));
    let result: Result<SkillOutput, Error> = Ok(output);
    let expectations = Expectations {
        not_contains: Some(vec!["world".to_string()]),
        ..Default::default()
    };

    let checks = check_all(&result, &expectations, Duration::from_millis(10));
    assert_eq!(checks.len(), 1);
    assert!(!checks[0].passed);
}

#[tokio::test]
async fn test_check_all_format_json_success() {
    let output = make_output(serde_json::json!({"valid": "json"}));
    let result: Result<SkillOutput, Error> = Ok(output);
    let expectations = Expectations {
        format: Some("json".to_string()),
        ..Default::default()
    };

    let checks = check_all(&result, &expectations, Duration::from_millis(10));
    assert_eq!(checks.len(), 1);
    assert!(checks[0].passed);
    assert_eq!(checks[0].check, "format=json");
}

#[tokio::test]
async fn test_check_all_format_json_failure() {
    // Test with invalid format type
    let output = make_output(serde_json::json!({"valid": "json"}));
    let result: Result<SkillOutput, Error> = Ok(output);
    let expectations = Expectations {
        format: Some("xml".to_string()),  // unsupported format
        ..Default::default()
    };

    let checks = check_all(&result, &expectations, Duration::from_millis(10));
    assert_eq!(checks.len(), 1);
    assert!(!checks[0].passed);
    assert!(checks[0].detail.as_ref().unwrap().contains("unknown format"));
}

#[tokio::test]
async fn test_check_all_duration_success() {
    let output = make_output(serde_json::json!({}));
    let result: Result<SkillOutput, Error> = Ok(output);
    let expectations = Expectations {
        max_duration_ms: Some(100),
        ..Default::default()
    };

    let checks = check_all(&result, &expectations, Duration::from_millis(50));
    assert_eq!(checks.len(), 1);
    assert!(checks[0].passed);
    assert_eq!(checks[0].check, "duration <= 100ms");
}

#[tokio::test]
async fn test_check_all_duration_failure() {
    let output = make_output(serde_json::json!({}));
    let result: Result<SkillOutput, Error> = Ok(output);
    let expectations = Expectations {
        max_duration_ms: Some(50),
        ..Default::default()
    };

    let checks = check_all(&result, &expectations, Duration::from_millis(100));
    assert_eq!(checks.len(), 1);
    assert!(!checks[0].passed);
    assert_eq!(checks[0].check, "duration <= 50ms");
    assert!(checks[0].detail.as_ref().unwrap().contains("took 100ms"));
}

#[tokio::test]
async fn test_check_all_output_matches_success() {
    let output = make_output(serde_json::json!({"message": "hello123"}));
    let result: Result<SkillOutput, Error> = Ok(output);
    let expectations = Expectations {
        output_matches: Some(r"hello\d+".to_string()),
        ..Default::default()
    };

    let checks = check_all(&result, &expectations, Duration::from_millis(10));
    assert_eq!(checks.len(), 1);
    assert!(checks[0].passed);
}

#[tokio::test]
async fn test_check_all_output_matches_failure() {
    let output = make_output(serde_json::json!({"message": "helloabc"}));
    let result: Result<SkillOutput, Error> = Ok(output);
    let expectations = Expectations {
        output_matches: Some(r"hello\d+".to_string()),
        ..Default::default()
    };

    let checks = check_all(&result, &expectations, Duration::from_millis(10));
    assert_eq!(checks.len(), 1);
    assert!(!checks[0].passed);
}

#[tokio::test]
async fn test_check_all_output_matches_invalid_regex() {
    let output = make_output(serde_json::json!({}));
    let result: Result<SkillOutput, Error> = Ok(output);
    let expectations = Expectations {
        output_matches: Some(r"[invalid(regex".to_string()),
        ..Default::default()
    };

    let checks = check_all(&result, &expectations, Duration::from_millis(10));
    assert_eq!(checks.len(), 1);
    assert!(!checks[0].passed);
    assert!(checks[0].detail.as_ref().unwrap().contains("invalid regex"));
}

#[test]
fn test_scenario_result_debug() {
    let result = ScenarioResult {
        skill: "test_skill".to_string(),
        scenario: "test_scenario".to_string(),
        passed: true,
        assertions: vec![],
        duration: Duration::from_millis(100),
        error: None,
    };
    
    // Just ensure Debug impl works
    let debug_str = format!("{:?}", result);
    assert!(debug_str.contains("test_skill"));
    assert!(debug_str.contains("test_scenario"));
}

#[test]
fn test_eval_report_print_pretty() {
    let result = ScenarioResult {
        skill: "test_skill".to_string(),
        scenario: "passing_scenario".to_string(),
        passed: true,
        assertions: vec![],
        duration: Duration::from_millis(100),
        error: None,
    };
    
    let report = EvalReport {
        results: vec![result],
        total: 1,
        passed: 1,
        failed: 0,
        duration: Duration::from_millis(100),
    };
    
    // Just ensure it doesn't panic
    report.print_pretty();
}

#[test]
fn test_eval_report_to_json() {
    let result = ScenarioResult {
        skill: "test_skill".to_string(),
        scenario: "test_scenario".to_string(),
        passed: false,
        assertions: vec![AssertionResult::fail_test("test", "detail")],
        duration: Duration::from_millis(100),
        error: Some("error message".to_string()),
    };

    let report = EvalReport {
        results: vec![result],
        total: 1,
        passed: 0,
        failed: 1,
        duration: Duration::from_millis(100),
    };

    let json = report.to_json();
    assert!(json.contains("\"total\":1"));
    assert!(json.contains("\"passed\":0"));
    assert!(json.contains("\"failed\":1"));
    assert!(json.contains("\"test_skill\""));
    assert!(json.contains("\"test_scenario\""));
}

#[tokio::test]
async fn test_eval_runner_with_mock_registry() {
    // Create a simple registry for testing
    let registry = SkillRegistry::new();
    let runner = orka_eval::EvalRunner::new(Arc::new(registry));
    
    // Create a temporary directory with an eval file
    let temp_dir = tempfile::tempdir().unwrap();
    let eval_path = temp_dir.path().join("test.eval.toml");
    
    let eval_content = r#"
skill = "nonexistent_skill"

[[scenarios]]
name = "test_scenario"
description = "Test that should fail since skill doesn't exist"

[scenarios.input]
arg1 = "value1"

[scenarios.expected]
is_ok = false
"#;
    
    std::fs::write(&eval_path, eval_content).unwrap();
    
    // Run the eval without filter to ensure file is found
    let report = runner.run_dir(temp_dir.path(), None).await.unwrap();
    
    // Verify report - skill doesn't exist so it should fail
    assert!(report.total >= 1, "should have run at least one scenario");
    // The scenario expects is_ok=false, and since skill doesn't exist, it should fail
    // So the assertion should pass (expecting failure, got failure)
    // But we're testing that the runner works, not the assertion logic
}

#[test]
fn test_eval_file_deserialize() {
    let content = r#"
skill = "test_skill"

[[scenarios]]
name = "scenario1"
description = "First test scenario"

[scenarios.input]
arg1 = "value1"
arg2 = 42

[scenarios.expected]
is_ok = true
contains = ["expected", "output"]
max_duration_ms = 1000
"#;
    
    let eval: EvalFile = toml::from_str(content).unwrap();
    assert_eq!(eval.skill, Some("test_skill".to_string()));
    assert_eq!(eval.scenarios.len(), 1);
    
    let scenario = &eval.scenarios[0];
    assert_eq!(scenario.name, "scenario1");
    assert_eq!(scenario.description, Some("First test scenario".to_string()));
    assert_eq!(scenario.input.get("arg1").unwrap().as_str(), Some("value1"));
    assert_eq!(scenario.input.get("arg2").unwrap().as_i64(), Some(42));
    
    let expected = &scenario.expected;
    assert_eq!(expected.is_ok, Some(true));
    assert_eq!(expected.contains, Some(vec!["expected".to_string(), "output".to_string()]));
    assert_eq!(expected.max_duration_ms, Some(1000));
}

#[test]
fn test_expectations_default() {
    let expectations = Expectations::default();
    assert!(expectations.contains.is_none());
    assert!(expectations.not_contains.is_none());
    assert!(expectations.format.is_none());
    assert!(expectations.is_ok.is_none());
    assert!(expectations.output_matches.is_none());
    assert!(expectations.max_duration_ms.is_none());
}
