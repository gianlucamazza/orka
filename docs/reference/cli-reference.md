# Orka CLI Reference

The `orka` command-line tool provides management commands for local setup,
server administration, agent operations, and observability.

This page is intentionally aligned with the current clap parser in
`crates/orka-cli/src/main.rs`. If a command is not listed here, do not assume it
is publicly available.

## Global Flags

All commands support the following global options to configure the target server:

*   `--server <url>` (or `ORKA_SERVER_URL` env var, default: `http://127.0.0.1:8080`)
*   `--adapter <url>` (or `ORKA_ADAPTER_URL` env var, default: `http://127.0.0.1:8081`)
*   `--api-key <key>` (or `ORKA_API_KEY` env var) for authenticated requests.

## Commands

### Setup and Lifecycle

```bash
orka init [--provider ...]       # Guided config bootstrap; writes or extends orka.toml
orka status                      # Server status (uptime, workers, adapters)
orka version                     # Show version (--check flags exits 1 if update is available)
orka update                      # Self-update the CLI binary
```

### Messaging and Agents

```bash
orka send "Hello"                # Send a one-off message (supports --session-id, --timeout)
orka chat                        # Interactive TUI session (supports --session-id)
orka workspace list|show <name>  # Manage and view configured workspaces
orka graph [--dot]               # Display the agent execution graph (text or Graphviz DOT)
orka a2a card                    # View the local agent card
orka a2a send <task>             # Send a delegated task via A2A
orka a2a stream <task>           # Stream a delegated task via A2A SSE
orka a2a tasks get|list|cancel   # Inspect or manage A2A task state
```

`orka workspace ...` operates on server-configured and built-in workspaces. Local root `SOUL.md` and `TOOLS.md` are part of local workspace discovery and prompt construction, not a separate server-managed workspace registry.

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
orka skill eval [--skill ...]    # Run .eval.toml scenarios against the skill registry
orka mcp-serve                   # Run Orka as an MCP server over stdio
```

### Observability

```bash
orka dashboard [--interval <s>]  # Launch the real-time TUI dashboard (health, metrics, sessions)
orka metrics [--filter] [--json] # Display Prometheus metrics
orka session list|show|delete    # Manage active sessions
orka dlq list|replay|purge       # Handle dead-letter queue entries
orka doctor                      # Run system diagnostics
orka doctor list                 # List available diagnostic checks
orka doctor explain <id>         # Explain a specific diagnostic check
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

## Notable Absences

- There is currently no public `orka research ...` command in the active CLI
  parser, even though the repository contains experimental/internal research
  documentation and implementation code.
- Older command names such as `orka health` or `orka ready` are not part of the
  current parser; use `orka status` or the HTTP health endpoints instead.
