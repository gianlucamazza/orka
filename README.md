# Orka

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

- `SOUL.md` — Agent personality and system prompt
- `IDENTITY.md` — Agent identity metadata

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

## Project Structure

```
orka/
├── orka-server/          # Binary composition root
├── crates/
│   ├── orka-core/        # Shared types, traits, errors
│   ├── orka-bus/         # Message bus (Redis Streams)
│   ├── orka-session/     # Session store
│   ├── orka-queue/       # Priority queue
│   ├── orka-worker/      # Worker pool & handlers
│   ├── orka-gateway/     # Inbound message gateway
│   ├── orka-observe/     # Domain event observability
│   ├── orka-skills/      # Skill registry & WASM plugins
│   ├── orka-sandbox/     # Code execution sandbox
│   ├── orka-memory/      # Key-value memory store
│   ├── orka-secrets/     # Secret management
│   ├── orka-auth/        # Authentication
│   ├── orka-workspace/   # Workspace loader & watcher
│   └── orka-adapter-*/   # Channel adapters
├── sdk/
│   └── orka-plugin-sdk/  # WASM plugin SDK
└── examples/
    └── hello-plugin/     # Example WASM plugin
```

## License

MIT
