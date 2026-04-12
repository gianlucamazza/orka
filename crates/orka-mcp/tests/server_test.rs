#![allow(missing_docs)]

use std::sync::Arc;

use orka_core::{testing::InMemorySecretManager, traits::SecretManager};
use orka_mcp::McpServer;
use orka_skills::{EchoSkill, SkillRegistry};
use serde_json::json;

fn response(value: Option<serde_json::Value>) -> serde_json::Value {
    let Some(response) = value else {
        panic!("expected MCP response");
    };
    response
}

fn json_array<'a>(value: &'a serde_json::Value, field: &str) -> &'a Vec<serde_json::Value> {
    let Some(array) = value[field].as_array() else {
        panic!("expected array field: {field}");
    };
    array
}

fn json_str<'a>(value: &'a serde_json::Value, field: &str) -> &'a str {
    let Some(text) = value[field].as_str() else {
        panic!("expected string field: {field}");
    };
    text
}

fn make_server() -> McpServer {
    let mut registry = SkillRegistry::new();
    registry.register(Arc::new(EchoSkill));
    let skills = Arc::new(registry);
    let secrets: Arc<dyn SecretManager> = Arc::new(InMemorySecretManager::new());
    McpServer::new(skills, secrets)
}

#[tokio::test]
async fn initialize_returns_protocol_version_and_capabilities() {
    let server = make_server();
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "initialize",
        "params": {
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": { "name": "test", "version": "0.1.0" }
        }
    });

    let response = response(server.handle_request(request).await);
    assert_eq!(response["jsonrpc"], "2.0");
    assert_eq!(response["id"], 1);
    assert_eq!(response["result"]["protocolVersion"], "2025-03-26");
    assert!(response["result"]["capabilities"]["tools"].is_object());
    assert_eq!(response["result"]["serverInfo"]["name"], "orka");
}

#[tokio::test]
async fn notifications_initialized_returns_none() {
    let server = make_server();
    let request = json!({
        "jsonrpc": "2.0",
        "method": "notifications/initialized"
    });

    let response = server.handle_request(request).await;
    assert!(response.is_none());
}

#[tokio::test]
async fn tools_list_returns_skills() {
    let server = make_server();
    let request = json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "tools/list",
        "params": {}
    });

    let response = response(server.handle_request(request).await);
    let tools = json_array(&response["result"], "tools");
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "echo");
    assert!(!json_str(&tools[0], "description").is_empty());
    assert!(tools[0]["inputSchema"].is_object());
}

#[tokio::test]
async fn tools_call_invokes_skill_and_returns_result() {
    let server = make_server();
    let request = json!({
        "jsonrpc": "2.0",
        "id": 3,
        "method": "tools/call",
        "params": {
            "name": "echo",
            "arguments": { "greeting": "hello" }
        }
    });

    let response = response(server.handle_request(request).await);
    assert_eq!(response["result"]["isError"], false);
    let content = json_array(&response["result"], "content");
    assert_eq!(content.len(), 1);
    assert_eq!(content[0]["type"], "text");
    let text = json_str(&content[0], "text");
    assert!(text.contains("greeting"));
    assert!(text.contains("hello"));
}

#[tokio::test]
async fn tools_call_unknown_tool_returns_error() {
    let server = make_server();
    let request = json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "tools/call",
        "params": {
            "name": "nonexistent",
            "arguments": {}
        }
    });

    let response = response(server.handle_request(request).await);
    assert_eq!(response["result"]["isError"], true);
    let content = json_array(&response["result"], "content");
    let text = json_str(&content[0], "text");
    assert!(text.contains("Error"));
}

#[tokio::test]
async fn unknown_method_returns_error_code() {
    let server = make_server();
    let request = json!({
        "jsonrpc": "2.0",
        "id": 5,
        "method": "some/unknown/method",
        "params": {}
    });

    let response = response(server.handle_request(request).await);
    assert_eq!(response["error"]["code"], -32601);
    assert!(json_str(&response["error"], "message").contains("Method not found"));
}
