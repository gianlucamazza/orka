use std::collections::HashMap;

use orka_mcp::{
    McpClient, McpContent, McpOAuthConfig, McpServerConfig, McpToolInfo, McpToolResult,
    McpTransportConfig,
};

#[test]
fn config_stdio_fields() {
    let config = McpServerConfig {
        name: "test-server".into(),
        transport: McpTransportConfig::Stdio {
            command: "echo".into(),
            args: vec!["hello".into(), "world".into()],
            env: HashMap::from([("KEY".into(), "VALUE".into())]),
        },
    };
    assert_eq!(config.name, "test-server");
    match &config.transport {
        McpTransportConfig::Stdio { command, args, env } => {
            assert_eq!(command, "echo");
            assert_eq!(args, &vec!["hello", "world"]);
            assert_eq!(env.get("KEY").unwrap(), "VALUE");
        }
        _ => panic!("expected Stdio transport"),
    }
}

#[test]
fn config_http_fields() {
    let config = McpServerConfig {
        name: "remote-server".into(),
        transport: McpTransportConfig::StreamableHttp {
            url: "https://tools.example.com/mcp".into(),
            auth: Some(McpOAuthConfig {
                token_url: "https://auth.example.com/token".into(),
                client_id: "orka-agent".into(),
                client_secret_env: "MCP_CLIENT_SECRET".into(),
                scopes: vec!["tools:read".into()],
            }),
        },
    };
    assert_eq!(config.name, "remote-server");
    match &config.transport {
        McpTransportConfig::StreamableHttp { url, auth } => {
            assert_eq!(url, "https://tools.example.com/mcp");
            let auth = auth.as_ref().expect("auth should be set");
            assert_eq!(auth.client_id, "orka-agent");
        }
        _ => panic!("expected StreamableHttp transport"),
    }
}

#[tokio::test]
async fn connect_nonexistent_command_fails() {
    let config = McpServerConfig {
        name: "bad".into(),
        transport: McpTransportConfig::Stdio {
            command: "/nonexistent/binary/that/does/not/exist".into(),
            args: vec![],
            env: HashMap::new(),
        },
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
