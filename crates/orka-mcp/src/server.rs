use std::sync::Arc;

use orka_core::{SkillContext, SkillInput, traits::SecretManager};
use orka_skills::SkillRegistry;
use serde_json::json;

/// MCP server that exposes Orka skills as JSON-RPC 2.0 tools.
pub struct McpServer {
    skills: Arc<SkillRegistry>,
    secrets: Arc<dyn SecretManager>,
}

impl McpServer {
    /// Create a new MCP server backed by the given skill registry and secret
    /// manager.
    pub fn new(skills: Arc<SkillRegistry>, secrets: Arc<dyn SecretManager>) -> Self {
        Self { skills, secrets }
    }

    /// Number of skills registered with this server.
    pub fn skill_count(&self) -> usize {
        self.skills.list().len()
    }

    /// Handle a JSON-RPC 2.0 request and return a response.
    pub async fn handle_request(&self, request: serde_json::Value) -> Option<serde_json::Value> {
        let id = request.get("id").cloned();
        let method = request["method"].as_str().unwrap_or("");

        match method {
            "initialize" => {
                let result = json!({
                    "protocolVersion": "2025-03-26",
                    "capabilities": {
                        "tools": { "listChanged": true }
                    },
                    "serverInfo": {
                        "name": "orka",
                        "version": env!("CARGO_PKG_VERSION")
                    }
                });
                Some(self.make_response(id, result))
            }
            "notifications/initialized" => None,
            "tools/list" => {
                let tools: Vec<serde_json::Value> = self
                    .skills
                    .list()
                    .iter()
                    .filter_map(|name| {
                        let skill = self.skills.get(name)?;
                        Some(json!({
                            "name": skill.name(),
                            "description": skill.description(),
                            "inputSchema": skill.schema().parameters
                        }))
                    })
                    .collect();
                Some(self.make_response(id, json!({ "tools": tools })))
            }
            "tools/call" => {
                let params = &request["params"];
                let name = params["name"].as_str().unwrap_or("");
                let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

                let args = arguments
                    .as_object()
                    .map(|obj| obj.iter().map(|(k, v)| (k.clone(), v.clone())).collect())
                    .unwrap_or_default();

                let input = SkillInput::new(args)
                    .with_context(SkillContext::new(self.secrets.clone(), None));

                match self.skills.invoke(name, input).await {
                    Ok(output) => {
                        let result = json!({
                            "content": [{ "type": "text", "text": output.data.to_string() }],
                            "isError": false
                        });
                        Some(self.make_response(id, result))
                    }
                    Err(e) => {
                        let result = json!({
                            "content": [{ "type": "text", "text": format!("Error: {e}") }],
                            "isError": true
                        });
                        Some(self.make_response(id, result))
                    }
                }
            }
            _ => Some(json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {
                    "code": -32601,
                    "message": format!("Method not found: {method}")
                }
            })),
        }
    }

    fn make_response(
        &self,
        id: Option<serde_json::Value>,
        result: serde_json::Value,
    ) -> serde_json::Value {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": result
        })
    }

    /// Run the server on stdio (JSON-RPC over stdin/stdout).
    pub async fn run_stdio(self: Arc<Self>) -> orka_core::Result<()> {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};

        let stdin = tokio::io::stdin();
        let mut stdout = tokio::io::stdout();
        let reader = BufReader::new(stdin);
        let mut lines = reader.lines();

        while let Ok(Some(line)) = lines.next_line().await {
            if line.is_empty()
                || line.starts_with("Content-Length:")
                || line.starts_with("content-length:")
            {
                continue;
            }

            let request: serde_json::Value = match serde_json::from_str(&line) {
                Ok(v) => v,
                Err(e) => {
                    tracing::warn!(%e, "failed to parse MCP request");
                    continue;
                }
            };

            if let Some(response) = self.handle_request(request).await {
                let mut out = serde_json::to_string(&response).unwrap_or_default();
                out.push('\n');
                if let Err(e) = stdout.write_all(out.as_bytes()).await {
                    tracing::error!(%e, "failed to write MCP response");
                    break;
                }
                let _ = stdout.flush().await;
            }
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use orka_core::testing::InMemorySecretManager;

    use super::*;

    fn test_server() -> McpServer {
        let skills = Arc::new(SkillRegistry::new());
        let secrets: Arc<dyn SecretManager> = Arc::new(InMemorySecretManager::new());
        McpServer::new(skills, secrets)
    }

    #[tokio::test]
    async fn handle_initialize_returns_server_info() {
        let server = test_server();
        let req = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize"});
        let resp = server.handle_request(req).await.unwrap();
        assert_eq!(resp["jsonrpc"], "2.0");
        assert_eq!(resp["id"], 1);
        assert_eq!(resp["result"]["serverInfo"]["name"], "orka");
        assert!(resp["result"]["protocolVersion"].is_string());
    }

    #[tokio::test]
    async fn handle_unknown_method_returns_error() {
        let server = test_server();
        let req = json!({"jsonrpc": "2.0", "id": 2, "method": "nonexistent/method"});
        let resp = server.handle_request(req).await.unwrap();
        assert_eq!(resp["error"]["code"], -32601);
        assert!(
            resp["error"]["message"]
                .as_str()
                .unwrap()
                .contains("nonexistent/method")
        );
    }

    #[tokio::test]
    async fn handle_tools_list_returns_empty_array() {
        let server = test_server();
        let req = json!({"jsonrpc": "2.0", "id": 3, "method": "tools/list"});
        let resp = server.handle_request(req).await.unwrap();
        assert_eq!(resp["result"]["tools"], json!([]));
    }
}
