# MCP (Model Context Protocol) Guide

Orka integrates with the Model Context Protocol (MCP) in two directions:

| Mode       | Description                                                                                    |
| ---------- | ---------------------------------------------------------------------------------------------- |
| **Client** | Orka connects to external MCP servers and exposes their tools to the agent                     |
| **Server** | Orka exposes its own skills as an MCP server consumed by clients such as Claude Code or Cursor |

---

## Orka as MCP Client

Configure external MCP servers under `[[mcp.servers]]` in `orka.toml`.
All tools discovered from a server are prefixed with the server name
(e.g., `github__create_issue`) to avoid name collisions.

### Stdio transport (default)

Launches a child process and communicates over stdin/stdout.

```toml
[[mcp.servers]]
name    = "github"
command = "npx"
args    = ["-y", "@modelcontextprotocol/server-github"]

[mcp.servers.env]
GITHUB_TOKEN = "ghp_..."  # injected into the child process
```

### Streamable HTTP transport (MCP spec 2025-03-26)

Connects to a remote HTTP endpoint with optional OAuth 2.1 authentication.

```toml
[[mcp.servers]]
name      = "my-remote-server"
transport = "streamable_http"
url       = "https://mcp.example.com/v1"
```

#### With OAuth 2.1 Client Credentials

```toml
[[mcp.servers]]
name      = "secure-server"
transport = "streamable_http"
url       = "https://mcp.example.com/v1"

[mcp.servers.auth]
token_url         = "https://auth.example.com/oauth/token"
client_id         = "orka-client"
client_secret_env = "MCP_CLIENT_SECRET"   # name of the env var holding the secret
scopes            = ["mcp:read", "mcp:tools"]
```

The token is cached in memory and refreshed 30 seconds before expiry.

---

## Orka as MCP Server

Expose Orka's registered skills as MCP tools consumable by Claude Code, Cursor,
or any MCP-compatible client.

### Stdio (recommended for local use)

```toml
[mcp.serve]
enabled   = true
transport = "stdio"
```

Start the server:

```bash
orka mcp-serve
```

#### Claude Code integration

Add Orka as a local MCP server in Claude Code's settings:

```json
{
  "mcpServers": {
    "orka": {
      "command": "orka",
      "args": ["mcp-serve"]
    }
  }
}
```

Or use the automated bridge (see [`docs/architecture.md`](architecture.md)):

```bash
# Start the claude-channel bridge
orka mcp-serve --bridge
```

### SSE transport

```toml
[mcp.serve]
enabled   = true
transport = "sse"
host      = "0.0.0.0"
port      = 8090
```

Clients connect to `http://localhost:8090/sse`.

---

## Skill Discovery

When Orka starts as an MCP server, it advertises all registered skills as MCP
tools. The tool name, description, and JSON schema are derived directly from
the `Skill` trait implementation.

Skills disabled in `tools.disabled` are **not** advertised.

---

## Troubleshooting

### "MCP server did not respond"

- Check that the child process is on `$PATH` (stdio transport).
- Check that the HTTP endpoint is reachable (HTTP transport).
- Increase log verbosity: `RUST_LOG=orka_mcp=debug orka-server`.

### "MCP error -32601: Method not found"

The remote server does not implement the requested method.
Check the server's MCP spec version against Orka's supported version (2025-03-26).

### OAuth token fetch fails

- Verify `MCP_CLIENT_SECRET` (or your configured env var) is set.
- Confirm the `token_url`, `client_id`, and `scopes` are correct.
- The client uses `grant_type=client_credentials`; ensure the server supports it.

### Tools not appearing in the agent

- Confirm the server name appears in logs: `loaded MCP server "github" with N tools`.
- Check that tool names don't clash (they are prefixed with the server name).
- Restart the server; MCP tool discovery happens at startup only.
