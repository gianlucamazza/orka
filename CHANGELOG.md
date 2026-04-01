# Changelog

All notable changes to Orka will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/), and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

### Changed

- **Crate consolidation** — reduced workspace from 46 to 40 members:
  - `orka-bus`, `orka-queue`, `orka-session`, `orka-conversation` merged into `orka-infra`
    (Redis-backed infrastructure services; all public APIs re-exported at crate root)
  - `orka-onboard` folded into `orka-cli` (was a single-dependent crate; now lives
    under `crates/orka-cli/src/onboard/`)
  - `orka-http` merged into `orka-web` (HTTP client skills now under `orka_web::http`;
    `HttpClientConfig`, `SsrfGuard`, and `create_http_skills` re-exported at crate root)
  - `orka-sandbox` merged into `orka-wasm` (sandbox execution now under
    `orka_wasm::sandbox`; `SandboxConfig`, `ProcessSandbox`, `WasmSandbox`, etc.
    re-exported at crate root)
- **`orka-config` feature gates** — 10 optional subsystem deps are now behind
  Cargo features: `telegram`, `discord`, `slack`, `whatsapp`, `chart`, `research`,
  `a2a`, `mcp`, `knowledge`, `experience`. The `default = ["full"]` meta-feature
  enables all of them for backwards compatibility. Partial builds
  (`cargo check -p orka-config --no-default-features`) now skip all unused
  subsystem crates, reducing incremental compile overhead.

### Removed

- Standalone crates `orka-bus`, `orka-queue`, `orka-session`, `orka-conversation`,
  `orka-onboard`, `orka-http`, `orka-sandbox` — functionality is fully preserved in
  the consolidated crates listed above; only import paths changed.

## [1.1.0] - 2026-04-01

### Added

- Dedicated mobile auth and pairing endpoints under `/mobile/v1/*` for QR-based
  first-device association, refresh-token rotation, and authenticated mobile
  sessions
- `orka mobile pair` CLI command with terminal QR rendering and pairing-status
  polling for first association between a signed-in CLI session and the mobile app
- Mobile auth service for device-scoped access/refresh token issuance with
  one-time, short-lived pairing sessions
- Public mobile API documentation covering pairing, refresh, conversation paging,
  and SSE event semantics

### Changed

- Mobile conversation endpoints now expose stable `limit`/`offset` pagination and
  explicit user-scoped authorization semantics
- Mobile SSE handling now distinguishes preview deltas from authoritative completed
  assistant messages and documents `stream_done` as transport completion only
- Mobile app onboarding now defaults to QR pairing with a manual pairing-URI
  fallback for simulators and restricted-permission environments

## [1.0.0] - 2026-03-29

### Added

- 14 new REST management endpoints: skills listing, scheduler CRUD, workspace inspection, graph topology, experience system (status/principles/distill), session management
- 8 new CLI commands: `skill`, `schedule`, `workspace`, `graph`, `experience`, `session`, `metrics`, `a2a`
- `SessionStore::list()` trait method with Redis SCAN and in-memory implementations
- `AgentGraph::nodes_iter()` / `edges_iter()` public accessors
- `Serialize` derive on `SoulFrontmatter`
- MCP serve now uses Redis-backed `SecretManager` when Redis is configured (fallback to in-memory)
- `NodeKindDef` config enum (`"agent"`, `"router"`, `"fan_out"`, `"fan_in"`) — graph topology is now
  fully declarative via `[[agents]] kind = "..."` in `orka.toml`
- `graph.entry` — explicit entry-point agent ID; falls back to the first `[[agents]]` entry
- Auto-derived `handoff_targets` — `agent`-kind nodes automatically advertise transfer/delegate tools
  for each of their outgoing graph edges; no manual `handoff_targets` configuration needed
- `history_filter` / `history_filter_n` per-agent fields — control how much conversation history
  is forwarded on handoff (`"full"` (default), `"last_n"`, `"none"`)
- Guardrails wired into `GraphExecutor` via `ExecutorDeps::guardrail` — three checkpoints per node:
  input, tool-call, and output; blocked requests short-circuit with an error response
- **Checkpointing** — `orka-checkpoint` crate: automatic per-node checkpoint saving, crash recovery
  via `GraphExecutor::resume()`, and REST API (`/api/v1/runs/{run_id}/checkpoints*`) for inspection
- **Human-in-the-Loop (HITL)** — `interrupt_before_tools` agent config pauses execution before
  specified tool calls; `POST /api/v1/runs/{run_id}/approve` re-enqueues for resumption,
  `POST /api/v1/runs/{run_id}/reject` marks the run as failed
- **Planning mode** — `planning_mode` per-agent config: `"always"` generates a structured plan via
  a dedicated LLM call before the first iteration; `"adaptive"` exposes plan tools for LLM-driven
  planning
- **History strategy** — `history_strategy` per-agent config: `"summarize"` calls the LLM to
  summarize dropped turns; `"rolling_window:<n>"` keeps the last *n* conversation turns with
  incremental background summarization
- **State reducers** — `[graph.reducers]` TOML config maps shared state slots to merge strategies
  (`append`, `sum`, `max`, `min`, `merge_object`, `last_write_wins`) enabling correct fan-out
  aggregation without coordinator agents
- **Multi-modal vision** — `ImageSource` (URL / Base64) and `ContentBlockInput::Image` added to
  `orka-llm`; dispatchers forward `image/*` media payloads as vision messages to Anthropic Claude
  and OpenAI providers; captions are appended as text blocks
- `orka-agent` crate: multi-agent graph executor with `AgentGraph`, `GraphExecutor`,
  fan-out/fan-in nodes, transfer/delegate handoffs, and termination policies
- `orka-wasm` crate: shared `WasmEngine` with module cache and per-instance limits,
  used by both `orka-sandbox` and `orka-skills` to avoid duplicate engine init
- `WorkerPoolGraph` in `orka-worker`: drop-in replacement for `WorkerPool` when
  multi-agent graph execution is desired
- Config: new `[[agents]]` list and `[graph]` topology section in `orka.toml` for
  declarative multi-agent deployments (`AgentDef`, `GraphDef`, `EdgeDef`)
- `RunId` type in `orka-core` for graph execution run tracking
- New `DomainEventKind` variants: `AgentDelegated`, `AgentCompleted`, `GraphCompleted`
- Discord adapter: slash command registration via `application_id`, image/file
  attachment support, command interaction handling
- Slack adapter: Block Kit formatted responses, file upload support, improved
  event deduplication
- WhatsApp adapter: media message support (image/audio/document), read receipts,
  improved webhook verification
- Plugin SDK: improved ergonomics, updated WASM ABI, and comprehensive API docs
  with ABI contract table and step-by-step quickstart
- `orka-llm`: new `error` module with structured `LlmError` enum (Network, Auth,
  RateLimit, ContextWindow, Provider, Parse, Stream) converting to `orka_core::Error`
- `orka-server`: adapter feature flags — `telegram`, `discord`, `slack`, `whatsapp`
  (all enabled by default); build with `--no-default-features` for a minimal server
- `/health/ready` endpoint now includes a Qdrant liveness check when `[knowledge]` is
  enabled, returning `"qdrant": "ok"` or an error message in the JSON response
- Privacy statement added to `README.md` clarifying that Orka collects no telemetry
  and all data remains within the user's own infrastructure

### Changed

- `orka-core::Error` variants `Worker`, `Sandbox`, `Observe`, `HttpClient`, `Llm`
  now carry a `#[source]` boxed error for full error chain preservation; new helper
  constructors (`worker()`, `sandbox()`, `observe()`, `http_client()`, `llm()`) accept
  any `std::error::Error` source; `*_msg()` variants for plain string messages
- `#[non_exhaustive]` added to 33 public config structs in `orka-core`, enabling
  future field additions without semver breaks
- `orka-sandbox` and `orka-skills` now depend on `orka-wasm` instead of `wasmtime`
  directly — single global module cache, reduced binary size
- Server instantiates shared `WasmEngine` and passes it to sandbox and plugin loader
- `WorkerPoolGraph` is the primary handler when `[graph]` is present in config
- `reqwest` workspace dep: added `multipart` feature
- `scraper` moved to a separate section in `Cargo.toml` for clarity
- circuit-breaker: replace `.unwrap()` with `.expect("mutex poisoned")` on mutex
  guards for clearer panic messages

### Removed

- `orka-router` crate: routing is now handled by `AgentGraph` in `orka-agent`

## [0.1.0] - 2026-03-16

### Added

- Multi-channel agent orchestration (Telegram, Discord, Slack, WhatsApp, custom HTTP/WebSocket)
- Priority queue with Redis Sorted Sets (Urgent, Normal, Background)
- LLM integration with Anthropic Claude and OpenAI (streaming support)
- Skill system with registry, schema validation, and WASM plugin support
- MCP (Model Context Protocol) server with JSON-RPC 2.0 over stdio
- A2A (Agent-to-Agent) protocol for inter-agent communication
- Agent router with prefix-based routing and delegation
- Workspace-based agent configuration with hot-reload (SOUL.md, TOOLS.md)
- Session management with Redis-backed storage
- Memory store (key-value per session)
- Secret management with AES-256-GCM encryption at rest
- Circuit breaker pattern for external service resilience
- Guardrails for input/output validation and content filtering
- Sandboxed code execution (process isolation and WASM)
- Knowledge base with RAG (Qdrant vector store, document ingestion)
- Cron-based task scheduler
- OS integration skills (filesystem, process, system info)
- HTTP request skill with SSRF protection
- Rate limiting and message deduplication at the gateway
- JWT and API key authentication
- OpenTelemetry tracing and Prometheus metrics
- Swagger UI for API documentation
- CLI tool for workspace management
- Docker Compose deployment
- Plugin SDK for WASM-based extensions
