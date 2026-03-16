use std::collections::HashMap;
use std::sync::Arc;

use orka_core::config::{SandboxConfig, SandboxLimitsConfig};
use orka_core::traits::Skill;
use orka_core::types::SkillInput;
use orka_sandbox::{
    ProcessSandbox, SandboxExecutor, SandboxLang, SandboxLimits, SandboxRequest, SandboxSkill,
};

fn default_config() -> SandboxConfig {
    SandboxConfig {
        backend: "process".into(),
        limits: SandboxLimitsConfig::default(),
    }
}

fn default_limits() -> SandboxLimits {
    SandboxLimits::default()
}

fn bash_request(code: &str) -> SandboxRequest {
    SandboxRequest {
        code: code.as_bytes().to_vec(),
        language: SandboxLang::Bash,
        stdin: None,
        env: HashMap::new(),
        limits: default_limits(),
    }
}

// --- ProcessSandbox tests ---

#[tokio::test]
async fn process_sandbox_bash_echo() {
    let sandbox = ProcessSandbox::new(&default_config());
    let req = bash_request("echo hello");
    let result = sandbox.execute(req).await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(String::from_utf8_lossy(&result.stdout).trim(), "hello");
    assert!(result.stderr.is_empty());
}

#[tokio::test]
async fn process_sandbox_bash_exit_code() {
    let sandbox = ProcessSandbox::new(&default_config());
    let req = bash_request("exit 42");
    let result = sandbox.execute(req).await.unwrap();
    assert_eq!(result.exit_code, 42);
}

#[tokio::test]
async fn process_sandbox_bash_stderr() {
    let sandbox = ProcessSandbox::new(&default_config());
    let req = bash_request("echo error_msg >&2");
    let result = sandbox.execute(req).await.unwrap();
    assert_eq!(
        String::from_utf8_lossy(&result.stderr).trim(),
        "error_msg"
    );
}

#[tokio::test]
async fn process_sandbox_bash_with_stdin() {
    let sandbox = ProcessSandbox::new(&default_config());
    let req = SandboxRequest {
        code: b"cat".to_vec(),
        language: SandboxLang::Bash,
        stdin: Some(b"piped input".to_vec()),
        env: HashMap::new(),
        limits: default_limits(),
    };
    let result = sandbox.execute(req).await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(
        String::from_utf8_lossy(&result.stdout).trim(),
        "piped input"
    );
}

#[tokio::test]
async fn process_sandbox_bash_with_env_vars() {
    let sandbox = ProcessSandbox::new(&default_config());
    let mut env = HashMap::new();
    env.insert("MY_VAR".into(), "my_value".into());
    let req = SandboxRequest {
        code: b"echo $MY_VAR".to_vec(),
        language: SandboxLang::Bash,
        stdin: None,
        env,
        limits: default_limits(),
    };
    let result = sandbox.execute(req).await.unwrap();
    assert_eq!(result.exit_code, 0);
    assert_eq!(
        String::from_utf8_lossy(&result.stdout).trim(),
        "my_value"
    );
}

#[tokio::test]
async fn process_sandbox_rejects_wasm() {
    let sandbox = ProcessSandbox::new(&default_config());
    let req = SandboxRequest {
        code: b"(module)".to_vec(),
        language: SandboxLang::Wasm,
        stdin: None,
        env: HashMap::new(),
        limits: default_limits(),
    };
    let result = sandbox.execute(req).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn process_sandbox_timeout() {
    let config = SandboxConfig {
        backend: "process".into(),
        limits: SandboxLimitsConfig {
            timeout_secs: 1,
            max_memory_bytes: 64 * 1024 * 1024,
            max_output_bytes: 1024 * 1024,
        },
    };
    let sandbox = ProcessSandbox::new(&config);
    let req = SandboxRequest {
        code: b"sleep 10".to_vec(),
        language: SandboxLang::Bash,
        stdin: None,
        env: HashMap::new(),
        limits: SandboxLimits {
            timeout: std::time::Duration::from_secs(1),
            max_memory_bytes: 64 * 1024 * 1024,
            max_output_bytes: 1024 * 1024,
        },
    };
    let result = sandbox.execute(req).await;
    // Either returns an error or a non-zero exit code
    match result {
        Err(_) => {}
        Ok(r) => assert_ne!(r.exit_code, 0, "timed-out process should not exit 0"),
    }
}

// --- SandboxSkill tests ---

fn make_skill() -> SandboxSkill {
    let sandbox = ProcessSandbox::new(&default_config());
    SandboxSkill::new(Arc::new(sandbox))
}

fn skill_input(args: Vec<(&str, serde_json::Value)>) -> SkillInput {
    let mut map = HashMap::new();
    for (k, v) in args {
        map.insert(k.to_string(), v);
    }
    SkillInput { args: map }
}

#[tokio::test]
async fn sandbox_skill_name_and_description() {
    let skill = make_skill();
    assert_eq!(skill.name(), "sandbox");
    assert!(!skill.description().is_empty());
}

#[tokio::test]
async fn sandbox_skill_schema_has_required_fields() {
    let skill = make_skill();
    let schema = skill.schema();
    let params = &schema.parameters;
    let props = params.get("properties").expect("schema must have properties");
    assert!(props.get("code").is_some(), "schema must have 'code'");
    assert!(
        props.get("language").is_some(),
        "schema must have 'language'"
    );
    assert!(
        props.get("timeout_secs").is_some(),
        "schema must have 'timeout_secs'"
    );
    let required = params.get("required").expect("schema must have 'required'");
    let req_arr = required.as_array().unwrap();
    assert!(req_arr.contains(&serde_json::json!("code")));
    assert!(req_arr.contains(&serde_json::json!("language")));
}

#[tokio::test]
async fn sandbox_skill_execute_bash() {
    let skill = make_skill();
    let input = skill_input(vec![
        ("code", serde_json::json!("echo skill_ok")),
        ("language", serde_json::json!("bash")),
    ]);
    let output = skill.execute(input).await.unwrap();
    let data = &output.data;
    assert_eq!(data["exit_code"], 0);
    assert!(data["stdout"].as_str().unwrap().contains("skill_ok"));
    assert!(data.get("duration_ms").is_some());
}

#[tokio::test]
async fn sandbox_skill_missing_code_arg() {
    let skill = make_skill();
    let input = skill_input(vec![("language", serde_json::json!("bash"))]);
    let result = skill.execute(input).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn sandbox_skill_missing_language_arg() {
    let skill = make_skill();
    let input = skill_input(vec![("code", serde_json::json!("echo hi"))]);
    let result = skill.execute(input).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn sandbox_skill_unsupported_language() {
    let skill = make_skill();
    let input = skill_input(vec![
        ("code", serde_json::json!("(module)")),
        ("language", serde_json::json!("wasm")),
    ]);
    let result = skill.execute(input).await;
    // wasm through process sandbox should fail
    assert!(result.is_err());
}

#[tokio::test]
async fn sandbox_skill_custom_timeout() {
    let skill = make_skill();
    let input = skill_input(vec![
        ("code", serde_json::json!("echo fast")),
        ("language", serde_json::json!("bash")),
        ("timeout_secs", serde_json::json!(5)),
    ]);
    let output = skill.execute(input).await.unwrap();
    let data = &output.data;
    assert_eq!(data["exit_code"], 0);
    assert!(data["stdout"].as_str().unwrap().contains("fast"));
    assert!(data.get("duration_ms").is_some());
}
