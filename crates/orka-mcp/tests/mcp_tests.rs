#![allow(missing_docs)]

use orka_mcp::{
    McpClient, McpContent, McpOAuthConfig, McpServerConfig, McpToolInfo, McpToolResult,
    McpTransportConfig,
};

fn some<'a, T>(value: Option<&'a T>, label: &str) -> &'a T {
    let Some(value) = value else {
        panic!("expected {label}");
    };
    value
}

#[test]
fn config_stdio_fields() {
    let config = McpServerConfig {
        name: "test-server".into(),
        transport: McpTransportConfig::stdio("echo")
            .args(["hello", "world"])
            .env("KEY", "VALUE")
            .build(),
    };
    assert_eq!(config.name, "test-server");
    match &config.transport {
        McpTransportConfig::Stdio {
            command, args, env, ..
        } => {
            assert_eq!(command, "echo");
            assert_eq!(args, &vec!["hello", "world"]);
            assert_eq!(some(env.get("KEY"), "KEY env var"), "VALUE");
        }
        _ => panic!("expected Stdio transport"),
    }
}

#[test]
fn config_http_fields() {
    let auth = McpOAuthConfig {
        token_url: "https://auth.example.com/token".into(),
        client_id: "orka-agent".into(),
        client_secret_env: "MCP_CLIENT_SECRET".into(),
        scopes: vec!["tools:read".into()],
    };
    let config = McpServerConfig {
        name: "remote-server".into(),
        transport: McpTransportConfig::http("https://tools.example.com/mcp")
            .auth(auth)
            .build(),
    };
    assert_eq!(config.name, "remote-server");
    match &config.transport {
        McpTransportConfig::StreamableHttp { url, auth } => {
            assert_eq!(url, "https://tools.example.com/mcp");
            let auth = some(auth.as_ref(), "HTTP auth config");
            assert_eq!(auth.client_id, "orka-agent");
        }
        _ => panic!("expected StreamableHttp transport"),
    }
}

#[tokio::test]
async fn connect_nonexistent_command_fails() {
    let config = McpServerConfig {
        name: "bad".into(),
        transport: McpTransportConfig::stdio("/nonexistent/binary/that/does/not/exist").build(),
    };
    let result = McpClient::connect(config).await;
    assert!(
        result.is_err(),
        "connect with nonexistent binary should fail"
    );
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
fn tool_info_deserialize() -> Result<(), serde_json::Error> {
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
    let info: McpToolInfo = serde_json::from_str(json)?;
    assert_eq!(info.name, "read_file");
    assert_eq!(info.description.as_deref(), Some("Read a file from disk"));
    assert!(info.input_schema.is_object());
    assert_eq!(info.input_schema["properties"]["path"]["type"], "string");
    Ok(())
}

#[test]
fn tool_result_deserialize_success() -> Result<(), serde_json::Error> {
    let json = r#"{
        "content": [{"type": "text", "text": "hello from tool"}],
        "is_error": false
    }"#;
    let result: McpToolResult = serde_json::from_str(json)?;
    assert!(!result.is_error);
    assert_eq!(result.content.len(), 1);
    match &result.content[0] {
        McpContent::Text { text } => assert_eq!(text, "hello from tool"),
    }
    Ok(())
}

#[test]
fn tool_result_deserialize_error() -> Result<(), serde_json::Error> {
    let json = r#"{
        "content": [{"type": "text", "text": "something went wrong"}],
        "is_error": true
    }"#;
    let result: McpToolResult = serde_json::from_str(json)?;
    assert!(result.is_error);
    match &result.content[0] {
        McpContent::Text { text } => assert!(text.contains("wrong")),
    }
    Ok(())
}
