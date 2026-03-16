use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use orka_core::{Error, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{oneshot, Mutex};
use tracing::{debug, warn};

use crate::config::McpServerConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolInfo {
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default, rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolResult {
    #[serde(default)]
    pub content: Vec<McpContent>,
    #[serde(default)]
    pub is_error: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum McpContent {
    #[serde(rename = "text")]
    Text { text: String },
}

pub struct McpClient {
    config: McpServerConfig,
    child: Mutex<Option<Child>>,
    stdin: Mutex<Option<tokio::process::ChildStdin>>,
    pending: Arc<Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>>,
    next_id: AtomicU64,
}

impl McpClient {
    pub async fn connect(config: McpServerConfig) -> Result<Self> {
        let mut cmd = Command::new(&config.command);
        cmd.args(&config.args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        for (k, v) in &config.env {
            cmd.env(k, v);
        }

        let mut child = cmd.spawn().map_err(|e| {
            Error::Other(format!("failed to spawn MCP server '{}': {e}", config.name))
        })?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Other("failed to capture MCP server stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Other("failed to capture MCP server stdout".into()))?;

        let pending: Arc<Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>> =
            Arc::new(Mutex::new(HashMap::new()));

        // Spawn reader task
        let pending_clone = pending.clone();
        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.is_empty() {
                    continue;
                }
                // Skip Content-Length headers (some MCP servers use HTTP-like framing)
                if line.starts_with("Content-Length:") || line.starts_with("content-length:") {
                    continue;
                }
                match serde_json::from_str::<serde_json::Value>(&line) {
                    Ok(msg) => {
                        if let Some(id) = msg.get("id").and_then(|v| v.as_u64()) {
                            let mut pending = pending_clone.lock().await;
                            if let Some(tx) = pending.remove(&id) {
                                let _ = tx.send(msg);
                            }
                        } else {
                            // Notification from server, log and ignore
                            debug!(server_notification = %line, "MCP server notification");
                        }
                    }
                    Err(e) => {
                        warn!(%e, line = %line, "failed to parse MCP server message");
                    }
                }
            }
        });

        let client = Self {
            config,
            child: Mutex::new(Some(child)),
            stdin: Mutex::new(Some(stdin)),
            pending,
            next_id: AtomicU64::new(1),
        };

        client.initialize().await?;

        Ok(client)
    }

    async fn send_request(
        &self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        let id = self.next_id.fetch_add(1, Ordering::SeqCst);

        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        let (tx, rx) = oneshot::channel();
        {
            let mut pending = self.pending.lock().await;
            pending.insert(id, tx);
        }

        let mut line = serde_json::to_string(&request)
            .map_err(|e| Error::Other(format!("failed to serialize MCP request: {e}")))?;
        line.push('\n');

        {
            let mut stdin_guard = self.stdin.lock().await;
            if let Some(ref mut stdin) = *stdin_guard {
                stdin
                    .write_all(line.as_bytes())
                    .await
                    .map_err(|e| Error::Other(format!("failed to write to MCP server: {e}")))?;
                stdin
                    .flush()
                    .await
                    .map_err(|e| Error::Other(format!("failed to flush MCP server stdin: {e}")))?;
            } else {
                return Err(Error::Other("MCP server stdin not available".into()));
            }
        }

        let response = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
            .await
            .map_err(|_| Error::Other(format!("MCP request '{}' timed out", method)))?
            .map_err(|_| Error::Other(format!("MCP request '{}' channel closed", method)))?;

        if let Some(error) = response.get("error") {
            let msg = error["message"].as_str().unwrap_or("unknown error");
            let code = error["code"].as_i64().unwrap_or(-1);
            return Err(Error::Other(format!("MCP error {code}: {msg}")));
        }

        Ok(response
            .get("result")
            .cloned()
            .unwrap_or(serde_json::Value::Null))
    }

    async fn initialize(&self) -> Result<()> {
        let params = serde_json::json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": {
                "name": "orka",
                "version": env!("CARGO_PKG_VERSION")
            }
        });

        let result = self.send_request("initialize", params).await?;
        debug!(server_name = ?result.get("serverInfo"), "MCP server initialized");

        // Send initialized notification (no response expected)
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/initialized",
        });
        let mut line = serde_json::to_string(&notification).unwrap();
        line.push('\n');
        let mut stdin_guard = self.stdin.lock().await;
        if let Some(ref mut stdin) = *stdin_guard {
            let _ = stdin.write_all(line.as_bytes()).await;
            let _ = stdin.flush().await;
        }

        Ok(())
    }

    pub async fn list_tools(&self) -> Result<Vec<McpToolInfo>> {
        let result = self
            .send_request("tools/list", serde_json::json!({}))
            .await?;
        let tools: Vec<McpToolInfo> = result
            .get("tools")
            .and_then(|t| serde_json::from_value(t.clone()).ok())
            .unwrap_or_default();
        debug!(count = tools.len(), server = %self.config.name, "discovered MCP tools");
        Ok(tools)
    }

    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<McpToolResult> {
        let result = self
            .send_request(
                "tools/call",
                serde_json::json!({
                    "name": name,
                    "arguments": arguments,
                }),
            )
            .await?;

        serde_json::from_value(result.clone())
            .map_err(|e| Error::Other(format!("failed to parse MCP tool result: {e}")))
    }

    pub fn server_name(&self) -> &str {
        &self.config.name
    }

    pub async fn shutdown(&self) {
        let mut child_guard = self.child.lock().await;
        if let Some(ref mut child) = *child_guard {
            let _ = child.kill().await;
        }
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        // Best-effort kill on drop
        if let Ok(mut guard) = self.child.try_lock() {
            if let Some(ref mut child) = *guard {
                let _ = child.start_kill();
            }
        }
    }
}
