# Orka

[![CI](https://github.com/gianlucamazza/orka/actions/workflows/ci.yml/badge.svg)](https://github.com/gianlucamazza/orka/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE-MIT)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE-APACHE)
[![Rust](https://img.shields.io/badge/rust-1.75%2B-orange.svg)](https://www.rust-lang.org)

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

## Features

- **Multi-channel messaging** — Telegram, Discord, Slack, WhatsApp, custom HTTP/WebSocket
- **Priority queue** — Redis Sorted Sets with Urgent / Normal / Background lanes
- **LLM integration** — Anthropic Claude and OpenAI with streaming support
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
- **CLI** — Workspace management tool

## Quick Start

### Prerequisites

- Rust 1.75+
- Redis 7+
- Docker (optional)

### With Docker Compose

```bash
docker-compose up
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

| Section           | Key           | Default                  | Description                           |
| ----------------- | ------------- | ------------------------ | ------------------------------------- |
| `server`          | `host`        | `127.0.0.1`              | Health endpoint bind address          |
| `server`          | `port`        | `8080`                   | Health endpoint port                  |
| `redis`           | `url`         | `redis://127.0.0.1:6379` | Redis connection URL                  |
| `worker`          | `concurrency` | `4`                      | Number of concurrent workers          |
| `session`         | `ttl_secs`    | `86400`                  | Session TTL in seconds (24h)          |
| `queue`           | `max_retries` | `3`                      | Max retries before dead-letter        |
| `adapters.custom` | `host`        | `127.0.0.1`              | Custom adapter bind address           |
| `adapters.custom` | `port`        | `8081`                   | Custom adapter port                   |
| `auth`            | `enabled`     | `false`                  | Enable API key authentication         |
| `sandbox`         | `backend`     | `process`                | Sandbox backend (`process` or `wasm`) |
| `logging`         | `level`       | `info`                   | Log level                             |
| `logging`         | `json`        | `false`                  | JSON log format                       |

Environment variables use `ORKA__` prefix with `__` as separator (e.g., `ORKA__REDIS__URL`).

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
cargo test --workspace -- --include-ignored

# Check formatting
cargo fmt --all -- --check

# Lint
cargo clippy --workspace --all-targets
```

## CLI

Orka includes a CLI tool for workspace management:

```bash
cargo run --bin orka-cli -- --help
```

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
│   ├── orka-router/          # Agent routing & delegation
│   ├── orka-guardrails/      # Input/output guardrails
│   ├── orka-circuit-breaker/ # Circuit breaker pattern
│   ├── orka-web/             # Web content extraction
│   ├── orka-os/              # OS integration skills
│   ├── orka-http/            # HTTP request skill
│   ├── orka-knowledge/       # RAG & vector knowledge base
│   ├── orka-scheduler/       # Cron-based task scheduler
│   ├── orka-cli/             # CLI tool
│   └── orka-adapter-*/       # Channel adapters
├── sdk/
│   └── orka-plugin-sdk/      # WASM plugin SDK
└── examples/
    └── hello-plugin/         # Example WASM plugin
```

## License

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)

at your option.
