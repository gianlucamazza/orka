# Orka

[![CI](https://github.com/gianlucamazza/orka/actions/workflows/ci.yml/badge.svg)](https://github.com/gianlucamazza/orka/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/License-MIT-blue.svg)](LICENSE-MIT)
[![License: Apache-2.0](https://img.shields.io/badge/License-Apache_2.0-blue.svg)](LICENSE-APACHE)
[![Rust](https://img.shields.io/badge/rust-1.96%2B-orange.svg)](https://www.rust-lang.org)

**What it is.** A self-hosted Rust runtime that accepts chat and HTTP messages, assigns them a queue priority, and runs them through bounded LLM agent workflows.

**Who it's for.** Teams routing Telegram, Discord, Slack, WhatsApp, or HTTP/WebSocket requests into those workflows — the same adapters and custom HTTP entry point `orka-server` starts.

**Status.** Pre-release, self-hosted, no SLA. Workspace crates set `publish = false` and are not on crates.io. Git tags exist through v1.11.0; there are no GitHub Releases yet.

Orka routes inbound messages from channel adapters and a custom HTTP/WebSocket endpoint through a Redis priority queue to LLM-powered agents. Agents can execute tools, optionally retrieve RAG context, and coordinate through multi-agent graphs. Extensibility includes WASM plugins and MCP/A2A clients. When `[experience].enabled` is set, the runtime can record run trajectories and distill them into prompt-injected principles.

<p align="center">
  <img src="demo/orka-chat.gif" alt="Orka CLI — interactive chat" width="720">
</p>

<details>
<summary>More demos</summary>

### Real-time dashboard

<p align="center">
  <img src="demo/orka-dashboard.gif" alt="Orka TUI dashboard" width="720">
</p>

### Server status &amp; skill listing

<p align="center">
  <img src="demo/orka-status.gif" alt="Orka status commands" width="720">
</p>

### One-shot message

<p align="center">
  <img src="demo/orka-send.gif" alt="Orka send" width="720">
</p>

</details>

Demo assets are regenerated with `just demo-check`, `just demo`, or `./scripts/demo.sh build <scenario>`. The asset contract and environment setup live in [demo/README.md](demo/README.md).

## Architecture

```mermaid
flowchart TD
    Clients["External Clients<br/>Telegram · Discord · Slack · WhatsApp · HTTP"]
    Auth["Auth Middleware<br/>orka-auth · API Key · JWT"]
    Adapters["Adapters<br/>Platform → Envelope conversion"]
    Bus["Message Bus<br/>Redis Streams"]
    GW["Gateway<br/>Dedup · Rate-limit · Session · Priority routing"]
    PQ["Priority Queue<br/>Redis Sorted Set<br/>Urgent > Normal > Background → DLQ"]
    Workers["Worker Pool (N concurrent)"]
    A2A["A2A Protocol<br/>orka-a2a · Agent card · JSON-RPC"]
    Eval["Eval Service<br/>orka-eval · Skill scenario testing"]

    subgraph Agent["Agent — GraphExecutor"]
        direction TB
        subgraph Context["  "]
            direction LR
            WS["Workspace<br/>SOUL.md · TOOLS.md"]
            GR["Guardrails<br/>Pre / Post"]
            Mem["Memory<br/>Key-Val store"]
            Exp["Experience<br/>Principles + Reflection"]
        end
        Loop["Agentic Loop<br/>LLM call → stream → tool calls → repeat"]
        LLM["LLM Router<br/>Anthropic · OpenAI · Ollama · Ollama Cloud<br/>Circuit Breaker per provider"]
        Skills["Skill Registry<br/>Shell · Code · Web · HTTP · Knowledge<br/>Schedule · MCP · Filesystem · System · WASM"]
        Loop --> LLM
        Loop --> Skills
    end

    Outbound["Outbound Bridge<br/>bus.publish('outbound')"]
    AdaptersOut["Adapters outbound<br/>Route reply by channel_id"]
    Platforms["Telegram · Discord · Slack · WhatsApp · HTTP"]
    Ext["External integrations<br/>Qdrant · MCP servers · Web / HTTP APIs"]
    BG["Background services<br/>Scheduler · Experience distillation · Observe (OTel / Prometheus)"]

    Clients --> Auth
    Auth --> Adapters
    Clients -.-> A2A
    Auth -.-> Eval
    Adapters -->|"publish('inbound')"| Bus
    Bus --> GW
    GW -->|"queue.push()"| PQ
    PQ -->|"queue.pop()"| Workers
    Workers --> Agent
    Skills --> Ext
    Agent --> Outbound
    Outbound --> AdaptersOut
    AdaptersOut --> Platforms
    Agent -.-> BG
```

For a detailed description of each subsystem and their interactions, see [docs/reference/architecture.md](docs/reference/architecture.md).

## Features

- **Multi-channel messaging** — Telegram, Discord, Slack, WhatsApp, and a custom HTTP/WebSocket adapter. Platform adapters start only when their Cargo feature, config section, and secrets are present; Slack is Events API (webhook), not Socket Mode.
- **Priority queue** — Redis Sorted Sets with Urgent / Normal / Background lanes
- **LLM integration** — Anthropic Claude, OpenAI, Ollama, Ollama Cloud, and Moonshot (OpenAI-compatible) with streaming support
- **Skill system** — Pluggable skills with JSON Schema validation and optional WASM plugins
- **MCP client and stdio server** — Outbound MCP tool bridges on `orka-server`; `orka mcp serve` exposes Orka over JSON-RPC 2.0 on stdio. The Streamable HTTP handler is not mounted on the main server.
- **A2A protocol** — Agent-to-Agent JSON-RPC routes mounted on `orka-server`
- **LLM model router** — Prefix-based model→provider routing with circuit-breaker fallback; graph nodes can also delegate between agents
- **Workspace config** — Hot-reloadable agent configuration (`SOUL.md`, `TOOLS.md`)
- **Knowledge base** — RAG with Qdrant vector store and document ingestion (requires Qdrant)
- **Constrained execution** — Subprocess resource limits and WASM capability sandboxing (not a full VM)
- **Guardrails** — Optional input/output validation and content filtering (`guardrails.enabled` defaults to off in code)
- **Checkpointing** — Per-node execution checkpoints with resume and human-in-the-loop approval
- **Multi-agent graphs** — Fan-out/fan-in, state reducers, planning mode, rolling-window history
- **Multi-modal** — Image attachments are forwarded to configured LLM providers that accept vision content
- **Circuit breaker** — Per-provider and per-skill breakers for external calls
- **Observability** — Prometheus `/metrics`, Swagger UI, optional OpenTelemetry export, optional append-only JSONL audit log
- **Auth and secrets** — Optional JWT/API key auth (off unless configured), AES-256-GCM secret encryption, SSRF checks on the HTTP skill
- **Scheduler** — Cron-based recurring tasks
- **Experience loop (opt-in)** — When `[experience].enabled` is true (default off), records run trajectories, can reflect them into principles, and can distill those principles offline into a Qdrant collection for later prompt injection
- **Soft skills** — Instruction-based `SKILL.md` files injected into the agent system prompt
- **MCP HTTP transport (client)** — Streamable HTTP (MCP spec 2025-03-26) with OAuth 2.1 Client Credentials when connecting to remote MCP servers
- **Skill evaluation** — TOML-based scenario runner for offline skill testing (`orka-eval`)
- **WASM Component Model** — Plugin SDK based on the WIT interface definition language
- **CLI** — Management CLI with a real-time TUI dashboard

## Quick Start

### Prerequisites

- Rust 1.96+
- Redis 7+
- Docker (optional)

### With Docker Compose

Copy `.env.example` to `.env` and fill in any required values, then:

```bash
docker compose up
```

### Manual Setup

```bash
# Start Redis (or Valkey — a fully compatible drop-in replacement)
docker run -d -p 6379:6379 redis:7-alpine

# Build and run
cargo build --release
./target/release/orka-server
```

### Native Installation (Linux with systemd)

```bash
# Dev setup — installs common deps, starts Redis/Valkey, runs cargo check
just setup

# systemd install — builds a release binary and installs a unit file
just install
systemctl enable --now orka-server

# Uninstall (preserves config and data)
just uninstall
```

`just setup` supports common `pacman`, `apt`, and `dnf` based development environments. Native Arch packaging is also available via `PKGBUILD`.

The server starts two endpoints:

- `http://localhost:8080` — Health endpoint
- `http://localhost:8081` — Custom HTTP/WebSocket adapter

### Send a message

```bash
curl -X POST http://localhost:8081/api/v1/message \
  -H "Content-Type: application/json" \
  -d '{"channel": "custom", "text": "Hello, Orka!"}'
```

## Configuration

Orka reads configuration from `orka.toml` and `ORKA_*` environment variables.
The repository root `orka.toml` is the canonical sample config kept in sync with
the current parser.

Until the wasmtime upgrade in [#47](https://github.com/gianlucamazza/orka/issues/47)
lands, do not set `sandbox.backend = "wasm"` (that path accepts caller-supplied
base64 modules) or point `plugins.dir` at modules you do not fully trust.
wasmtime 42.0.2 is affected by RUSTSEC-2026-0321/0322/0323/0327. The stock
config (`sandbox.backend = "process"`, `plugins.dir` unset) is not exposed.
See [SECURITY.md](SECURITY.md#known-issues).

For a complete reference of all configuration options, see the [Configuration Guide](docs/reference/configuration.md).

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
| `MOONSHOT_API_KEY`           | Moonshot provider fallback                              |
| `OPENAI_API_KEY`             | OpenAI provider fallback                                |
| `TAVILY_API_KEY`             | Tavily web search key                                   |
| `BRAVE_API_KEY`              | Brave web search key                                    |
| `RUST_LOG`                   | Overrides `logging.level` via tracing `EnvFilter`       |
| `ORKA_GIT_SHA`               | Git SHA embedded at build time                          |
| `ORKA_BUILD_DATE`            | Build date embedded at build time                       |
| `ORKA_NO_UPDATE_CHECK`       | Disable automatic update check on CLI startup           |

Config fields can also be overridden via `ORKA__<SECTION>__<KEY>` (e.g., `ORKA__REDIS__URL`).

### API Endpoints

**Server (`:8080`):**

| Method   | Path                            | Description                                          |
| -------- | ------------------------------- | ---------------------------------------------------- |
| `GET`    | `/health`                       | Health check                                         |
| `GET`    | `/health/live`                  | Liveness probe                                       |
| `GET`    | `/health/ready`                 | Readiness probe                                      |
| `GET`    | `/metrics`                      | Prometheus metrics (when `observe.backend = "otlp"` or `prometheus`) |
| `GET`    | `/docs`                         | Swagger UI (OpenAPI)                                 |
| `GET`    | `/api/v1/version`               | Version info                                         |
| `GET`    | `/api/v1/info`                  | Server info and feature flags                        |
| `GET`    | `/api/v1/dlq`                   | List dead-letter entries                             |
| `DELETE` | `/api/v1/dlq`                   | Purge dead-letter queue                              |
| `POST`   | `/api/v1/dlq/{id}/replay`       | Replay a dead-letter entry                           |
| `GET`    | `/api/v1/skills`                | List registered skills                               |
| `GET`    | `/api/v1/soft-skills`           | List discovered soft skills                          |
| `GET`    | `/api/v1/skills/{name}`         | Skill detail with schema                             |
| `POST`   | `/api/v1/eval`                  | Run eval scenarios                                   |
| `GET`    | `/api/v1/schedules`             | List scheduled tasks                                 |
| `POST`   | `/api/v1/schedules`             | Create a schedule                                    |
| `DELETE` | `/api/v1/schedules/{id}`        | Delete a schedule                                    |
| `GET`    | `/api/v1/workspaces`            | List server workspaces                               |
| `GET`    | `/api/v1/workspaces/{name}`     | Workspace detail                                     |
| `GET`    | `/api/v1/graph`                 | Agent graph topology                                 |
| `GET`    | `/api/v1/experience/status`     | Experience system status                             |
| `GET`    | `/api/v1/experience/principles` | Retrieve learned principles                          |
| `POST`   | `/api/v1/experience/distill`    | Trigger principle distillation                       |
| `GET`    | `/api/v1/sessions`              | List active sessions                                 |
| `GET`    | `/api/v1/sessions/{id}`         | Session detail                                       |
| `DELETE` | `/api/v1/sessions/{id}`         | Delete a session                                     |
| `GET`    | `/api/v1/runs/{run_id}/checkpoints`        | List checkpoint IDs for a run           |
| `GET`    | `/api/v1/runs/{run_id}/checkpoints/latest` | Most recent checkpoint                  |
| `GET`    | `/api/v1/runs/{run_id}/status`             | Run status from latest checkpoint       |
| `POST`   | `/api/v1/runs/{run_id}/approve`            | Approve an interrupted run (HITL)       |
| `POST`   | `/api/v1/runs/{run_id}/reject`             | Reject an interrupted run (HITL)        |

**Adapter (`:8081`):**

| Method | Path              | Description          |
| ------ | ----------------- | -------------------- |
| `POST` | `/api/v1/message` | Send a message       |
| `GET`  | `/api/v1/ws`      | WebSocket connection |
| `GET`  | `/api/v1/health`  | Adapter health       |

> **Hot-reload**: Orka watches the `.env` file for changes. API key updates trigger automatic LLM client refresh without restart.

## Workspaces

Agent behavior is configured through workspace files:

- `SOUL.md` — Agent personality and system prompt (markdown with YAML frontmatter)
- `TOOLS.md` — Tool usage guidelines for the LLM (plain markdown)

Runtime parameters (model, tokens, etc.) live in `orka.toml` under `[[agents]]` and `[tools]`.

Orka currently uses a two-tier workspace model:

- **Local repository workspace** — the root `SOUL.md` and `TOOLS.md` discovered from the current working tree by local tooling and CLI flows.
- **Built-in runtime workspaces** — distributable workspace files under `workspaces/`, used by runtime registration and installation.

These are related but not interchangeable concepts. The root files define the local repo's active workspace context, while `workspaces/` stores built-in workspace content shipped with the system.

Workspaces support hot-reloading via filesystem watcher.

## Documentation

Use the [Documentation Index](docs/README.md) as the canonical navigation hub.
The table below is a curated subset of the most common entrypoints.

| Guide                                          | Description                                               |
| ---------------------------------------------- | --------------------------------------------------------- |
| [Architecture](docs/reference/architecture.md) | End-to-end message flow and subsystem overview            |
| [Deployment](docs/reference/deployment.md)     | Docker, bare-metal, systemd, reverse proxy, observability |
| [Configuration](docs/reference/configuration.md) | Detailed reference for `orka.toml` and env vars         |
| [CLI Reference](docs/reference/cli-reference.md) | Current commands and flags exposed by the `orka` binary |
| [Prompt Architecture](docs/guides/agents.md)   | Guide to the prompt pipeline, templates, and Workspaces   |
| [Skill Development](docs/guides/skill-development.md) | Built-in, WASM, and soft skills; eval framework     |
| [WASM Tutorial](docs/guides/tutorials/build-a-wasm-plugin.md) | Step-by-step guide to writing WebAssembly plugins |
| [MCP Guide](docs/reference/mcp-guide.md)       | MCP client/server, HTTP transport, OAuth                  |
| [Experience System](docs/guides/experience-system.md) | Opt-in trajectory recording, reflection, distillation |
| [Eval Framework](docs/guides/eval-guide.md)    | TOML scenario runner for offline skill testing            |
| [Security](SECURITY.md)                        | Vulnerability reporting, hardening checklist              |

Internal-only or preview material lives under `docs/internal/` or is explicitly
marked as preview when it documents code paths that are not currently exposed by
the public CLI.

Companion docs that sit outside `docs/` are also indexed from [docs/README.md](docs/README.md), including examples, packaging, SDK, tooling, tests, and selected crate-local READMEs.

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

The `orka` command-line tool provides management commands for server administration, agent operations, and observability.

For a complete list of commands and global options, see the [CLI Reference](docs/reference/cli-reference.md).

## Project Structure

Curated overview of the main repository areas; this is intentionally not an
exhaustive listing of every workspace package.

```
orka/
├── crates/
│   ├── orka-core/            # Shared types, traits, errors
│   ├── orka-infra/           # Message bus, queue, session, conversation (Redis)
│   ├── orka-auth/            # JWT and API key authentication
│   ├── orka-worker/          # Worker pool & handlers
│   ├── orka-gateway/         # Inbound message gateway
│   ├── orka-observe/         # Domain event observability
│   ├── orka-skills/          # Skill registry & execution
│   ├── orka-coding/          # Coding delegation skills (Claude Code, Codex, OpenCode)
│   ├── orka-workspace/       # Workspace loader & watcher
│   ├── orka-llm/             # LLM providers (Anthropic, Moonshot, OpenAI, Ollama, Ollama Cloud)
│   ├── orka-mcp/             # Model Context Protocol server
│   ├── orka-a2a/             # Agent-to-Agent protocol
│   ├── orka-guardrails/      # Input/output guardrails
│   ├── orka-circuit-breaker/ # Circuit breaker pattern
│   ├── orka-web/             # Web search, page reading, and HTTP client skills
│   ├── orka-os/              # OS integration skills
│   ├── orka-knowledge/       # RAG & vector knowledge base
│   ├── orka-scheduler/       # Cron-based task scheduler
│   ├── orka-experience/      # Optional trajectory / principle loop
│   ├── orka-agent/           # Agent orchestration and routing
│   ├── orka-wasm/            # WASM runtime utilities, Component Model, and code sandbox
│   ├── orka-checkpoint/      # Execution checkpointing and crash recovery
│   ├── orka-eval/            # Skill evaluation framework (TOML scenarios)
│   ├── orka-cli/             # CLI tool
│   └── orka-adapter-*/       # Channel adapters
├── sdk/
│   ├── orka-plugin-sdk/          # WASM module plugin SDK
│   ├── orka-plugin-sdk-component/ # WASM Component Model plugin SDK (WIT-based)
│   └── hello-plugin/             # Example WASM plugin
├── docs/
│   ├── reference/            # Stable reference docs
│   ├── guides/               # How-tos and development guides
│   └── internal/             # Internal analysis and planning docs
├── evals/                    # Built-in evaluation scenarios (*.eval.toml)
├── examples/                 # Runnable examples and integration samples
├── deploy/                   # systemd service unit, sysusers, tmpfiles, sudoers
├── workspaces/               # Built-in workspace prompt files used by the runtime
├── tools/claude-channel/     # MCP bridge for Claude Code ↔ Orka integration (TypeScript/Bun)
├── tests/                    # Reserved for end-to-end and cross-crate integration tests
└── scripts/                  # install.sh and setup-dev.sh
```

## Root Conventions

The repository root is intentionally kept as the composition layer of the project.

- Keep product code in `crates/`, not in ad hoc top-level folders.
- Keep stable entrypoints and repo-wide manifests in root: `Cargo.toml`, `orka.toml`, `Justfile`, `Dockerfile`, `docker-compose*.yml`.
- Keep repository-wide policy and tooling config in root: `.editorconfig`, `.rustfmt.toml`, `deny.toml`, `release.toml`, `cliff.toml`, `.pre-commit-config.yaml`.
- Keep user-facing examples, SDKs, docs, deploy assets, and scripts in their dedicated top-level directories.
- Treat `workspaces/` as a runtime data area for built-in workspace prompt files, not as a general development folder.
- Treat root `SOUL.md` and `TOOLS.md` as the local repository workspace, distinct from the built-in workspaces stored under `workspaces/`.
- Use `scripts/` for shell automation and installation flows.
- Use `tools/` for standalone helper utilities with their own runtime or toolchain.
- Reserve `tests/` for end-to-end or cross-crate tests. Crate-local unit and integration tests should stay inside each crate.

Some root files exist for local tool integration rather than core runtime architecture, such as `.mcp.json`, `.claude/`, and `GEMINI.md`. They should remain documented and intentional, but they are not the primary model for organizing product code.

## Privacy

Orka does not ship a vendor telemetry or analytics pipeline. Traffic still leaves your host when you configure LLM providers, channel adapters, MCP servers, web tools, or observability exporters. The CLI may query GitHub for release tags unless `ORKA_NO_UPDATE_CHECK` is set.

- **LLM API calls** go from your process to the provider you configure (Anthropic, Moonshot, OpenAI, Ollama, Ollama Cloud, etc.). Orka is not an LLM proxy. Prompt and response text can still appear in local logs, traces, or audit records depending on settings. Optional guardrails and the experience loop can send additional content to an LLM you configure.
- **Messages and sessions** are stored in the Redis instance you run.
- **WASM plugins** run with host-enforced memory and CPU limits. Network is disabled by default and must be granted in plugin capabilities.
- **Knowledge and experience vectors** are stored in the Qdrant instance you configure.

You choose which channels, providers, and exporters are configured. Untrusted input still needs auth and guardrails.

## License

Licensed under either of

- [Apache License, Version 2.0](LICENSE-APACHE)
- [MIT License](LICENSE-MIT)

at your option.
