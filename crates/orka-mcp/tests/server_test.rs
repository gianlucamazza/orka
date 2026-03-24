use std::sync::Arc;

use orka_core::{
    testing::{EchoSkill, InMemorySecretManager},
    traits::SecretManager,
};
use orka_mcp::McpServer;
use orka_skills::SkillRegistry;
use serde_json::json;

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

    let response = server.handle_request(request).await.unwrap();
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

    let response = server.handle_request(request).await.unwrap();
    let tools = response["result"]["tools"].as_array().unwrap();
    assert_eq!(tools.len(), 1);
    assert_eq!(tools[0]["name"], "echo");
    assert!(!tools[0]["description"].as_str().unwrap().is_empty());
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

    let response = server.handle_request(request).await.unwrap();
    assert_eq!(response["result"]["isError"], false);
    let content = response["result"]["content"].as_array().unwrap();
    assert_eq!(content.len(), 1);
    assert_eq!(content[0]["type"], "text");
    let text = content[0]["text"].as_str().unwrap();
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

    let response = server.handle_request(request).await.unwrap();
    assert_eq!(response["result"]["isError"], true);
    let content = response["result"]["content"].as_array().unwrap();
    let text = content[0]["text"].as_str().unwrap();
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

    let response = server.handle_request(request).await.unwrap();
    assert_eq!(response["error"]["code"], -32601);
    assert!(
        response["error"]["message"]
            .as_str()
            .unwrap()
            .contains("Method not found")
    );
}
