# Orka CLI Reference

The `orka` command-line tool provides a full suite of management commands for server administration, agent operations, and observability.

## Global Flags

All commands support the following global options to configure the target server:

*   `--server <url>` (or `ORKA_SERVER_URL` env var, default: `http://127.0.0.1:8080`)
*   `--adapter <url>` (or `ORKA_ADAPTER_URL` env var, default: `http://127.0.0.1:8081`)
*   `--api-key <key>` (or `ORKA_API_KEY` env var) for authenticated requests.

## Commands

### Health and Status

```bash
orka health                      # General server health check
orka status                      # Server status (uptime, workers, adapters)
orka ready                       # Readiness probe (exits 1 if not ready)
orka version                     # Show version (--check flags exits 1 if update is available)
orka update                      # Self-update the CLI binary
```

### Messaging and Agents

```bash
orka send "Hello"                # Send a one-off message (supports --session-id, --timeout)
orka chat                        # Interactive TUI session (supports --session-id)
orka workspace list|show <name>  # Manage and view configured workspaces
orka graph show [--dot]          # Display the agent execution graph (text or Graphviz DOT)
orka a2a card|send               # View A2A agent card or send a delegated task
```

### Configuration and Secrets

```bash
orka config check                # Validate the orka.toml schema
orka config migrate              # Perform schema migration (--dry-run to preview)
orka secret set|get|list|delete  # Manage AES-256-GCM encrypted secrets
orka sudo check                  # Verify sudoers configuration for allowed OS commands
```

### Skills and Capabilities

```bash
orka skill list|describe <name>  # List registered skills or display their JSON schema
orka mcp-serve                   # Run Orka as an MCP server over stdio
```

### Observability

```bash
orka dashboard [--interval <s>]  # Launch the real-time TUI dashboard (health, metrics, sessions)
orka metrics [--filter] [--json] # Display Prometheus metrics
orka session list|show|delete    # Manage active sessions
orka dlq list|replay|purge       # Handle dead-letter queue entries
```

### Scheduling and Learning

```bash
orka schedule list|create|delete # Manage cron-based scheduled tasks
orka experience status|principles|distill # Monitor the self-learning system and manage principles
```

### System

```bash
orka completions <shell>         # Generate auto-completions for bash/zsh/fish
```
