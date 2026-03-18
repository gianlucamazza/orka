# Orka

[![CI](https://github.com/gianlucamazza/orka/actions/workflows/ci.yml/badge.svg)](https://github.com/gianlucamazza/orka/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE-MIT)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE-APACHE)
[![Rust](https://img.shields.io/badge/rust-1.85%2B-orange.svg)](https://www.rust-lang.org)

An agent orchestration platform built in Rust. Orka routes messages from external channels through a priority queue to AI-powered agent handlers, with support for skills, sandboxed code execution, and workspace-based configuration.

## Architecture

```
External Clients
       │
 ┌─────▼─────┐
 │  Adapters  │  HTTP/WS, Telegram, Discord, ...
 └─────┬─────┘
       │
 ┌─────▼─────┐
 │  Message   │  Redis Streams (pub/sub with consumer groups)
 │    Bus     │
 └─────┬─────┘
       │
 ┌─────▼─────┐
 │  Gateway   │  Session resolution, message routing
 └─────┬─────┘
       │
 ┌─────▼─────┐
 │  Priority  │  Redis Sorted Sets (Urgent > Normal > Background)
 │   Queue    │
 └─────┬─────┘
       │
 ┌─────▼─────┐
 │  Worker    │  Concurrent handlers with skill registry
 │   Pool     │
 └─────┬─────┘
       │
 ┌─────▼─────┐
 │  Outbound  │  Route replies back through adapters
 │  Bridge    │
 └─────┘
```

For a detailed description of each subsystem and their interactions, see [docs/architecture.md](docs/architecture.md).

## Features

- **Multi-channel messaging** — Telegram, Discord, Slack, WhatsApp, custom HTTP/WebSocket
- **Priority queue** — Redis Sorted Sets with Urgent / Normal / Background lanes
- **LLM integration** — Anthropic Claude, OpenAI, and Ollama (OpenAI-compatible) with streaming support
- **Skill system** — Pluggable skills with schema validation and WASM plugin support
- **MCP server** — Model Context Protocol over JSON-RPC 2.0
- **A2A protocol** — Agent-to-Agent communication
- **Agent router** — Prefix-based routing with delegation
- **Workspace config** — Hot-reloadable agent configuration (SOUL.md, TOOLS.md)
- **Knowledge base** — RAG with Qdrant vector store and document ingestion
- **Sandboxed execution** — Process isolation and WASM sandboxing
- **Guardrails** — Input/output validation and content filtering
- **Circuit breaker** — Resilience pattern for external services
- **Observability** — OpenTelemetry tracing, Prometheus metrics, Swagger UI
- **Security** — JWT/API key auth, AES-256-GCM secret encryption, SSRF protection
- **Scheduler** — Cron-based recurring tasks
- **Self-learning** — Trajectory recording, principle reflection, and offline distillation
- **CLI** — Workspace management tool

## Quick Start

### Prerequisites

- Rust 1.85+
- Redis 7+
- Docker (optional)

### With Docker Compose

Copy `.env.example` to `.env` and fill in any required values, then:

```bash
docker compose up
```

### Manual Setup

```bash
# Start Redis
docker run -d -p 6379:6379 redis:7-alpine

# Build and run
cargo build --release
./target/release/orka-server
```

### Native Installation (Arch Linux)

```bash
# Dev setup — installs deps, starts Redis, runs cargo check
just setup

# Production install — builds release binary, installs systemd service
just install
systemctl enable --now orka-server

# Uninstall (preserves config and data)
just uninstall
```

The server starts two endpoints:

- `http://localhost:8080` — Health endpoint
- `http://localhost:8081` — Custom HTTP/WebSocket adapter

### Send a message

```bash
curl -X POST http://localhost:8081/api/message \
  -H "Content-Type: application/json" \
  -d '{"channel": "custom", "text": "Hello, Orka!"}'
```

## Configuration

Orka reads configuration from `orka.toml` and `ORKA_*` environment variables.

| Section                  | Key                       | Default                  | Description                                |
| ------------------------ | ------------------------- | ------------------------ | ------------------------------------------ |
| `server`                 | `host`                    | `127.0.0.1`              | Health endpoint bind address               |
| `server`                 | `port`                    | `8080`                   | Health endpoint port                       |
| `redis`                  | `url`                     | `redis://127.0.0.1:6379` | Redis connection URL                       |
| `worker`                 | `concurrency`             | `4`                      | Number of concurrent workers               |
| `session`                | `ttl_secs`                | `86400`                  | Session TTL in seconds (24h)               |
| `queue`                  | `max_retries`             | `3`                      | Max retries before dead-letter             |
| `adapters.custom`        | `host`                    | `127.0.0.1`              | Custom adapter bind address                |
| `adapters.custom`        | `port`                    | `8081`                   | Custom adapter port                        |
| `adapters.telegram`      | `bot_token_secret`        | —                        | Secret path for bot token                  |
| `adapters.telegram`      | `mode`                    | `polling`                | `polling` or `webhook`                     |
| `adapters.telegram`      | `parse_mode`              | `HTML`                   | Outbound text format                       |
| `adapters.telegram`      | `webhook_url`             | —                        | Public URL for webhook mode                |
| `adapters.telegram`      | `webhook_port`            | `8443`                   | Local port for webhook listener            |
| `auth`                   | `enabled`                 | `false`                  | Enable API key authentication              |
| `sandbox`                | `backend`                 | `process`                | Sandbox backend (`process` or `wasm`)      |
| `logging`                | `level`                   | `info`                   | Log level                                  |
| `logging`                | `json`                    | `false`                  | JSON log format                            |
| `agent`                  | `id`                      | `orka-default`           | Agent identifier                           |
| `agent`                  | `max_iterations`          | `10`                     | Max agentic loop iterations per turn       |
| `agent`                  | `heartbeat_interval_secs` | `30`                     | Streaming heartbeat interval               |
| `llm`                    | `timeout_secs`            | `30`                     | LLM request timeout                        |
| `llm`                    | `max_tokens`              | `8192`                   | Default max output tokens                  |
| `llm.providers`          | `name`                    | —                        | Provider name (array of provider configs)  |
| `knowledge`              | `enabled`                 | `true`                   | Enable RAG/knowledge base                  |
| `knowledge.vector_store` | `provider`                | `qdrant`                 | Vector store backend                       |
| `knowledge.vector_store` | `url`                     | `http://localhost:6334`  | Qdrant endpoint                            |
| `scheduler`              | `enabled`                 | `true`                   | Enable cron scheduler                      |
| `scheduler`              | `poll_interval_secs`      | `5`                      | Scheduler polling interval                 |
| `web`                    | `search_provider`         | `none`                   | Web search backend (`tavily` or `searxng`) |
| `os`                     | `enabled`                 | `true`                   | Enable OS integration skills               |
| `os`                     | `permission_level`        | `admin`                  | OS skill permission level                  |
| `http`                   | `enabled`                 | `true`                   | Enable HTTP request skill                  |
| `plugins`                | `dir`                     | `plugins`                | Directory for WASM plugin files            |
| `guardrails`             | `blocked_keywords`        | `[]`                     | Keywords that trigger message blocking     |
| `guardrails`             | `pii_filter`              | `false`                  | Enable PII redaction                       |
| `mcp.servers`            | `name`                    | —                        | MCP server name (array of server configs)  |
| `mcp.servers`            | `command`                 | —                        | Command to launch MCP server               |
| `mcp.serve`              | `enabled`                 | `false`                  | Expose Orka as an MCP server               |
| `mcp.serve`              | `transport`               | `stdio`                  | `stdio` or `sse`                           |
| `bus`                    | `backend`                 | `redis`                  | Message bus backend                        |
| `bus`                    | `block_ms`                | `5000`                   | XREADGROUP BLOCK timeout (ms)              |
| `bus`                    | `batch_size`              | `10`                     | Messages per read batch                    |
| `memory`                 | `backend`                 | `auto`                   | `redis`, `memory`, or `auto`               |
| `session`                | `backend`                 | `auto`                   | `redis`, `memory`, or `auto`               |
| `queue`                  | `backend`                 | `auto`                   | `redis`, `memory`, or `auto`               |
| `observe`                | `backend`                 | `log`                    | `log`, `redis`, or `otel`                  |
| `agent`                  | `max_history_entries`     | `50`                     | Max conversation turns kept in context     |
| `agent`                  | `skill_timeout_secs`      | `120`                    | Per-skill execution timeout                |
| `agent`                  | `temperature`             | —                        | LLM sampling temperature (0.0–2.0)         |
| `agent`                  | `thinking_budget_tokens`  | —                        | Anthropic extended thinking budget         |
| `agent`                  | `reasoning_effort`        | —                        | OpenAI o-series: `low`, `medium`, `high`   |
| `experience`             | `enabled`                 | `false`                  | Enable self-learning experience loop       |
| `experience`             | `reflect_on`              | `failures`               | `failures`, `all`, or `sampled`            |
| `experience`             | `max_principles`          | `5`                      | Max principles injected into system prompt |
| `a2a`                    | `enabled`                 | `false`                  | Enable Agent-to-Agent protocol             |
| `os`                     | `sensitive_env_patterns`  | glob list                | Env var patterns redacted from tool output |
| `os`                     | `allowed_commands`        | `[]`                     | Explicit command allow-list for OS skills  |

For a complete reference, see [`orka.toml`](orka.toml).

### Environment Variables

| Variable                     | Description                                             |
| ---------------------------- | ------------------------------------------------------- |
| `ORKA_CONFIG`                | Path to config file (default: `./orka.toml`)            |
| `ORKA_ENV_FILE`              | Path to `.env` file for hot-reload                      |
| `ORKA_ENV` / `APP_ENV`       | `production` requires encryption key for secrets        |
| `ORKA_SECRET_ENCRYPTION_KEY` | 32-byte hex key for AES-256-GCM secret encryption       |
| `ORKA_HOST_HOSTNAME`         | Override hostname in system info                        |
| `ORKA_SERVER_URL`            | CLI: server endpoint (default `http://127.0.0.1:8080`)  |
| `ORKA_ADAPTER_URL`           | CLI: adapter endpoint (default `http://127.0.0.1:8081`) |
| `ORKA_API_KEY`               | CLI: API key for authenticated requests                 |
| `ANTHROPIC_API_KEY`          | Anthropic provider fallback                             |
| `OPENAI_API_KEY`             | OpenAI provider fallback                                |
| `TAVILY_API_KEY`             | Tavily web search key                                   |
| `BRAVE_API_KEY`              | Brave web search key                                    |
| `RUST_LOG`                   | Overrides `logging.level` via tracing `EnvFilter`       |

Config fields can also be overridden via `ORKA__<SECTION>__<KEY>` (e.g., `ORKA__REDIS__URL`).

> **Hot-reload**: Orka watches the `.env` file for changes. API key updates trigger automatic LLM client refresh without restart.

## Workspaces

Agent behavior is configured through workspace files:

- `SOUL.md` — Agent personality and system prompt (markdown with YAML frontmatter)
- `TOOLS.md` — Tool usage guidelines for the LLM (plain markdown)

Runtime parameters (model, tokens, heartbeat, etc.) live in `orka.toml` under `[agent]` and `[tools]`.

Workspaces support hot-reloading via filesystem watcher.

## Development

```bash
# Run all tests
cargo test --workspace

# Run with Redis integration tests
cargo test --workspace -- --ignored

# Check formatting
cargo fmt --all -- --check

# Lint
cargo clippy --workspace --all-targets
```

## CLI

```bash
orka health                      # Server health check
orka status                      # Server status (uptime, workers, adapters)
orka ready                       # Readiness probe (exit 1 if not ready)
orka send "Hello"                # Send a message (--session-id, --timeout)
orka chat                        # Interactive session (--session-id)
orka dlq list|replay|purge       # Dead letter queue management
orka secret set|get|list|delete  # Encrypted secret management
orka config check                # Validate orka.toml
orka config migrate              # Schema migration (--dry-run)
orka sudo check                  # Verify sudoers for allowed commands
orka mcp-serve                   # Run as MCP server (stdio)
orka completions <shell>         # Generate completions (bash/zsh/fish)
```

Global flags: `--server <url>`, `--adapter <url>`, `--api-key <key>` (or env vars above).

## Project Structure

```
orka/
├── orka-server/              # Binary composition root
├── crates/
│   ├── orka-core/            # Shared types, traits, errors
│   ├── orka-bus/             # Message bus (Redis Streams)
│   ├── orka-auth/            # JWT and API key authentication
│   ├── orka-session/         # Session store
│   ├── orka-queue/           # Priority queue
│   ├── orka-worker/          # Worker pool & handlers
│   ├── orka-gateway/         # Inbound message gateway
│   ├── orka-observe/         # Domain event observability
│   ├── orka-skills/          # Skill registry & execution
│   ├── orka-sandbox/         # Code execution sandbox (process + WASM)
│   ├── orka-memory/          # Key-value memory store
│   ├── orka-secrets/         # Secret management (AES-256-GCM)
│   ├── orka-workspace/       # Workspace loader & watcher
│   ├── orka-llm/             # LLM providers (Anthropic, OpenAI)
│   ├── orka-mcp/             # Model Context Protocol server
│   ├── orka-a2a/             # Agent-to-Agent protocol
│   ├── orka-guardrails/      # Input/output guardrails
│   ├── orka-circuit-breaker/ # Circuit breaker pattern
│   ├── orka-web/             # Web content extraction
│   ├── orka-os/              # OS integration skills
│   ├── orka-http/            # HTTP request skill
│   ├── orka-knowledge/       # RAG & vector knowledge base
│   ├── orka-scheduler/       # Cron-based task scheduler
│   ├── orka-experience/      # Self-learning experience system
│   ├── orka-agent/           # Agent orchestration and routing
│   ├── orka-wasm/            # WASM runtime utilities
│   ├── orka-cli/             # CLI tool
│   └── orka-adapter-*/       # Channel adapters
├── sdk/
│   └── orka-plugin-sdk/      # WASM plugin SDK
└── examples/
    └── hello-plugin/         # Example WASM plugin
```

## Privacy

Orka does not collect telemetry, usage data, or analytics of any kind. No data leaves your infrastructure unless you explicitly configure it to do so.

- **LLM API calls** are made directly from your deployment to the provider you configure (Anthropic, OpenAI, Ollama, etc.). Orka does not proxy or inspect these requests.
- **Messages and sessions** are stored in your own Redis instance. Nothing is sent to third-party services without your configuration.
- **WASM plugins** run in a sandboxed environment with explicit memory and CPU limits. They cannot make outbound network calls unless the host grants access.
- **Knowledge base** (RAG) data is stored in your own Qdrant instance.

You are in full control of what enters and exits the system.

## License

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)

at your option.
