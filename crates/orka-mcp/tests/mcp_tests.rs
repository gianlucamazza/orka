use orka_mcp::{McpClient, McpContent, McpServerConfig, McpToolInfo, McpToolResult};
use std::collections::HashMap;

#[test]
fn config_fields() {
    let config = McpServerConfig {
        name: "test-server".into(),
        command: "echo".into(),
        args: vec!["hello".into(), "world".into()],
        env: HashMap::from([("KEY".into(), "VALUE".into())]),
    };
    assert_eq!(config.name, "test-server");
    assert_eq!(config.command, "echo");
    assert_eq!(config.args, vec!["hello", "world"]);
    assert_eq!(config.env.get("KEY").unwrap(), "VALUE");
}

#[test]
fn config_deserialize_full() {
    let json = r#"{
        "name": "my-mcp",
        "command": "/usr/bin/tool",
        "args": ["--port", "8080"],
        "env": {"TOKEN": "abc123", "DEBUG": "1"}
    }"#;
    let config: McpServerConfig = serde_json::from_str(json).expect("deserialize config");
    assert_eq!(config.name, "my-mcp");
    assert_eq!(config.command, "/usr/bin/tool");
    assert_eq!(config.args, vec!["--port", "8080"]);
    assert_eq!(config.env.len(), 2);
    assert_eq!(config.env["TOKEN"], "abc123");
    assert_eq!(config.env["DEBUG"], "1");
}

#[test]
fn config_deserialize_defaults() {
    // args and env have #[serde(default)], so they can be omitted
    let json = r#"{"name": "minimal", "command": "cat"}"#;
    let config: McpServerConfig = serde_json::from_str(json).expect("deserialize minimal config");
    assert_eq!(config.name, "minimal");
    assert_eq!(config.command, "cat");
    assert!(config.args.is_empty());
    assert!(config.env.is_empty());
}

#[tokio::test]
async fn connect_nonexistent_command_fails() {
    let config = McpServerConfig {
        name: "bad".into(),
        command: "/nonexistent/binary/that/does/not/exist".into(),
        args: vec![],
        env: HashMap::new(),
    };
    let result = McpClient::connect(config).await;
    assert!(result.is_err(), "connect with nonexistent binary should fail");
    let err = match result {
        Err(e) => e.to_string(),
        Ok(_) => panic!("expected error"),
    };
    assert!(
        err.contains("failed to spawn"),
        "error should mention spawn failure, got: {err}"
    );
}

#[test]
fn tool_info_deserialize() {
    let json = r#"{
        "name": "read_file",
        "description": "Read a file from disk",
        "inputSchema": {
            "type": "object",
            "properties": {
                "path": {"type": "string"}
            },
            "required": ["path"]
        }
    }"#;
    let info: McpToolInfo = serde_json::from_str(json).expect("deserialize tool info");
    assert_eq!(info.name, "read_file");
    assert_eq!(info.description.as_deref(), Some("Read a file from disk"));
    assert!(info.input_schema.is_object());
    assert_eq!(info.input_schema["properties"]["path"]["type"], "string");
}

#[test]
fn tool_result_deserialize_success() {
    let json = r#"{
        "content": [{"type": "text", "text": "hello from tool"}],
        "is_error": false
    }"#;
    let result: McpToolResult = serde_json::from_str(json).expect("deserialize tool result");
    assert!(!result.is_error);
    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        McpContent::Text { text } => assert_eq!(text, "hello from tool"),
    }
}

#[test]
fn tool_result_deserialize_error() {
    let json = r#"{
        "content": [{"type": "text", "text": "something went wrong"}],
        "is_error": true
    }"#;
    let result: McpToolResult = serde_json::from_str(json).expect("deserialize error result");
    assert!(result.is_error);
    match &result.content[0] {
        McpContent::Text { text } => assert!(text.contains("wrong")),
    }
}
