use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use orka_core::{Error, Result};
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::{Mutex, oneshot};
use tracing::{debug, warn};

use crate::config::{McpServerConfig, McpTransportConfig};
use crate::http_transport::HttpTransport;
use crate::oauth::OAuthClient;

/// Metadata for a single tool advertised by an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolInfo {
    /// Tool name as registered on the MCP server.
    pub name: String,
    /// Human-readable description of the tool.
    #[serde(default)]
    pub description: Option<String>,
    /// JSON schema describing the tool's input parameters.
    #[serde(default, rename = "inputSchema")]
    pub input_schema: serde_json::Value,
}

/// Result returned by an MCP tool call.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpToolResult {
    /// Content fragments produced by the tool.
    #[serde(default)]
    pub content: Vec<McpContent>,
    /// `true` if the tool reported an error.
    #[serde(default)]
    pub is_error: bool,
}

/// A content fragment within an [`McpToolResult`].
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum McpContent {
    /// Plain-text content.
    #[serde(rename = "text")]
    Text {
        /// The text payload.
        text: String,
    },
}

type PendingMap = Arc<Mutex<HashMap<u64, oneshot::Sender<serde_json::Value>>>>;

enum Transport {
    Stdio {
        child: Mutex<Option<Child>>,
        stdin: Mutex<Option<tokio::process::ChildStdin>>,
        pending: PendingMap,
    },
    Http(HttpTransport),
}

/// JSON-RPC 2.0 client that talks to an external MCP server via stdio or HTTP.
pub struct McpClient {
    name: String,
    transport: Transport,
    next_id: AtomicU64,
}

impl McpClient {
    /// Connect to an MCP server using the provided configuration.
    pub async fn connect(config: McpServerConfig) -> Result<Self> {
        let name = config.name.clone();
        match config.transport {
            McpTransportConfig::Stdio { command, args, env, working_dir } => {
                Self::connect_stdio(name, command, args, env, working_dir).await
            }
            McpTransportConfig::StreamableHttp { url, auth } => Self::connect_http(name, url, auth),
        }
    }

    async fn connect_stdio(
        name: String,
        command: String,
        args: Vec<String>,
        env: HashMap<String, String>,
        working_dir: Option<std::path::PathBuf>,
    ) -> Result<Self> {
        let mut cmd = Command::new(&command);
        cmd.args(&args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null());
        for (k, v) in &env {
            cmd.env(k, v);
        }
        if let Some(dir) = &working_dir {
            cmd.current_dir(dir);
        }

        let mut child = cmd
            .spawn()
            .map_err(|e| Error::Other(format!("failed to spawn MCP server '{name}': {e}")))?;

        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| Error::Other("failed to capture MCP server stdin".into()))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| Error::Other("failed to capture MCP server stdout".into()))?;

        let pending: PendingMap = Arc::new(Mutex::new(HashMap::new()));
        let pending_clone = pending.clone();

        tokio::spawn(async move {
            let reader = BufReader::new(stdout);
            let mut lines = reader.lines();
            while let Ok(Some(line)) = lines.next_line().await {
                if line.is_empty()
                    || line.starts_with("Content-Length:")
                    || line.starts_with("content-length:")
                {
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
                            debug!(notification = %line, "MCP server notification");
                        }
                    }
                    Err(e) => warn!(%e, line = %line, "failed to parse MCP server message"),
                }
            }
        });

        let client = Self {
            name,
            transport: Transport::Stdio {
                child: Mutex::new(Some(child)),
                stdin: Mutex::new(Some(stdin)),
                pending,
            },
            next_id: AtomicU64::new(1),
        };

        client.initialize().await?;
        Ok(client)
    }

    fn connect_http(
        name: String,
        url: String,
        auth: Option<crate::config::McpOAuthConfig>,
    ) -> Result<Self> {
        let http = reqwest::Client::new();
        let oauth = auth
            .map(|a| OAuthClient::from_config(http.clone(), &a))
            .transpose()?;
        Ok(Self {
            name,
            transport: Transport::Http(HttpTransport::new(http, url, oauth)),
            next_id: AtomicU64::new(1),
        })
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

        match &self.transport {
            Transport::Stdio { stdin, pending, .. } => {
                let (tx, rx) = oneshot::channel();
                pending.lock().await.insert(id, tx);

                let mut line = serde_json::to_string(&request)
                    .map_err(|e| Error::Other(format!("failed to serialize MCP request: {e}")))?;
                line.push('\n');

                {
                    let mut guard = stdin.lock().await;
                    if let Some(ref mut w) = *guard {
                        w.write_all(line.as_bytes()).await.map_err(|e| {
                            Error::Other(format!("failed to write to MCP server: {e}"))
                        })?;
                        w.flush().await.map_err(|e| {
                            Error::Other(format!("failed to flush MCP server stdin: {e}"))
                        })?;
                    } else {
                        return Err(Error::Other("MCP server stdin not available".into()));
                    }
                }

                let resp = tokio::time::timeout(std::time::Duration::from_secs(30), rx)
                    .await
                    .map_err(|_| Error::Other(format!("MCP request '{method}' timed out")))?
                    .map_err(|_| Error::Other(format!("MCP request '{method}' channel closed")))?;

                if let Some(error) = resp.get("error") {
                    let msg = error["message"].as_str().unwrap_or("unknown error");
                    let code = error["code"].as_i64().unwrap_or(-1);
                    return Err(Error::Other(format!("MCP error {code}: {msg}")));
                }

                Ok(resp
                    .get("result")
                    .cloned()
                    .unwrap_or(serde_json::Value::Null))
            }
            Transport::Http(http) => http.send(request).await,
        }
    }

    async fn send_notification(&self, method: &str) {
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
        });
        match &self.transport {
            Transport::Stdio { stdin, .. } => {
                let mut line = serde_json::to_string(&notification).unwrap();
                line.push('\n');
                let mut guard = stdin.lock().await;
                if let Some(ref mut w) = *guard {
                    let _ = w.write_all(line.as_bytes()).await;
                    let _ = w.flush().await;
                }
            }
            Transport::Http(_) => {
                // HTTP MCP servers do not expect stdio-style notifications.
            }
        }
    }

    async fn initialize(&self) -> Result<()> {
        let params = serde_json::json!({
            "protocolVersion": "2025-03-26",
            "capabilities": {},
            "clientInfo": {
                "name": "orka",
                "version": env!("CARGO_PKG_VERSION")
            }
        });

        let result = self.send_request("initialize", params).await?;
        debug!(server_info = ?result.get("serverInfo"), server = %self.name, "MCP server initialized");

        self.send_notification("notifications/initialized").await;
        Ok(())
    }

    /// Request the list of tools available on the MCP server.
    pub async fn list_tools(&self) -> Result<Vec<McpToolInfo>> {
        let result = self
            .send_request("tools/list", serde_json::json!({}))
            .await?;
        let tools: Vec<McpToolInfo> = result
            .get("tools")
            .and_then(|t| serde_json::from_value(t.clone()).ok())
            .unwrap_or_default();
        debug!(count = tools.len(), server = %self.name, "discovered MCP tools");
        Ok(tools)
    }

    /// Invoke a named tool with the given arguments on the MCP server.
    pub async fn call_tool(
        &self,
        name: &str,
        arguments: serde_json::Value,
    ) -> Result<McpToolResult> {
        let result = self
            .send_request(
                "tools/call",
                serde_json::json!({ "name": name, "arguments": arguments }),
            )
            .await?;
        serde_json::from_value(result.clone())
            .map_err(|e| Error::Other(format!("failed to parse MCP tool result: {e}")))
    }

    /// Return the configured server name.
    pub fn server_name(&self) -> &str {
        &self.name
    }

    /// Shut down the MCP server connection.
    pub async fn shutdown(&self) {
        match &self.transport {
            Transport::Stdio { child, .. } => {
                let mut guard = child.lock().await;
                if let Some(ref mut c) = *guard
                    && let Err(e) = c.kill().await
                {
                    warn!(%e, server = %self.name, "failed to kill MCP server process");
                }
            }
            Transport::Http(_) => {} // Stateless; nothing to shut down.
        }
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        if let Transport::Stdio { child, .. } = &self.transport
            && let Ok(mut guard) = child.try_lock()
            && let Some(ref mut c) = *guard
            && let Err(e) = c.start_kill()
        {
            tracing::warn!(%e, "failed to kill MCP server on drop");
        }
    }
}
