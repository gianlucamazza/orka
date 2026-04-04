# Changelog

All notable changes to Orka will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/), and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [1.5.0] - 2026-04-04

### Added

- Mobile API: `POST /mobile/v1/conversations/{id}/cancel` — cancels an in-progress
  generation by signalling the shared `SessionCancelTokens` map; worker pool and HTTP
  router now share the same token map via a new `WorkerPool::with_session_cancel_tokens`
  builder method
- Mobile API: `DELETE /mobile/v1/conversations/{id}/messages/{message_id}` — removes a
  single message from a conversation transcript; backed by a new `delete_message` method
  on the `ConversationStore` trait with Redis (LRANGE→filter→DEL+RPUSH) and in-memory
  implementations
- Mobile API: `POST /mobile/v1/conversations/{id}/retry` — retries the last failed
  generation: verifies `status == Failed`, removes trailing assistant messages, and
  re-publishes the last user message to the inbound bus
- `Conversation` now carries `pinned: bool` and `tags: Vec<String>` fields; both use
  `#[serde(default)]` so existing stored conversations deserialize without migration
- `PATCH /mobile/v1/conversations/{id}` extended to accept `title`, `pinned`, and `tags`
  in addition to `archived`; all fields are optional, at least one must be present
- `POST /mobile/v1/conversations/{id}/read` registered in the OpenAPI spec with a
  `#[utoipa::path]` annotation and schema for `MarkReadRequest`

### Changed

- OpenAPI annotation on `GET /mobile/v1/conversations/{id}/messages` corrected to reflect
  cursor-based pagination (`after`, `before`, `limit`) instead of the stale `offset` docs;
  `x-next-cursor` and `x-prev-cursor` response headers are now documented
- `docs/guides/mobile-client.md` updated: cursor pagination semantics, full table of new
  endpoints, and all 10 SSE event types with payload shapes (`typing_started`,
  `thinking_delta`, `tool_exec_start`, `tool_exec_end`, `agent_switch` were missing)
- Integration test `mobile_message_list_supports_limit_and_offset` replaced with
  `mobile_message_list_cursor_pagination` which covers forward and backward cursor traversal

## [1.4.0] - 2026-04-04

### Added

- Redis-backed observe sink configuration and metrics, plus Redis pool retry helpers and
  integration coverage for gateway dedup/rate limiting and scheduler stores
- Per-user mobile API rate limiting with explicit 429 coverage in the mobile test suite;
  read (GET/HEAD) and write (POST/PATCH/PUT/DELETE) requests now use separate buckets —
  300 req/min and 60 req/min respectively (previously a single 120 req/min bucket)
- JWT/authenticator validation coverage, including RSA/RS256 verification tests
- Test coverage for `create_experience_service` initialization (enabled/disabled paths),
  gateway stale rate-counter cleanup at the 10 000-entry threshold, `RedisEventSink`
  interval-triggered flush when buffer is below batch size, and scheduler `max_concurrent`
  semaphore enforcement under parallel load
- A repeatable public demo pipeline that records live scenarios and renders `gif`, `mp4`,
  and `webm` assets through `just demo*` and `scripts/demo.sh`

### Changed

- Replaced deprecated `serde_yaml 0.9` with `serde_yml 0.0.12` across the workspace
  (`orka-skills`, `orka-workspace`); the API is a drop-in rename of `from_str`
- Config loading now has narrower subsystem boundaries and a dedicated
  `orka-config::subsystem_config` surface, reducing cross-crate coupling in server/bootstrap
  and agent graph wiring
- Workspace-related failures now use simpler shared error paths instead of ad-hoc variants across
  chart, prompt, mobile-auth, and server call-sites
- Archived conversation filtering in the Redis conversation store was simplified to reduce
  pagination/filter drift
- `LlmRouter` can now try configured fallback providers when the primary provider circuit
  breaker is open instead of failing immediately
- Agent `FanOut` nodes now support optional concurrency limits, with graph/config wiring
  and executor coverage
- CLI chat/send no longer attach `workspace:cwd` when targeting a remote Orka instance, and
  agent parsing now drops empty tool-call names before skill dispatch
- Experience service initialization now respects the configured vector-store backend instead
  of always forcing Qdrant

### Fixed

- Resolved all outstanding clippy warnings: `assigning_clones` (use `clone_from` in CLI and
  bootstrap converters), `items_after_statements` (elevate test-only structs in `jwt.rs`),
  `match_wildcard_for_single_variants` (explicit `VectorStoreBackend::Qdrant` arm), and
  replaced `unwrap_err()` with `is_err_and` in auth config tests

## [1.3.0] - 2026-04-03

### Added

- `llm_call_timeout_secs` (default: 120 s) and `max_run_secs` (default: none) config fields
  on `AgentConfig` / `Agent` — allows capping individual LLM calls and total wall-clock run
  time per agent invocation (`orka-core`, `orka-agent`)

### Changed

- Agent now returns explicit user-visible error messages when a run is interrupted by
  `max_turns`, LLM-call timeout, or wall-clock timeout instead of silently returning an
  empty response (`orka-agent`)
- `sanitize_tool_result_history` now also strips `ToolUse` blocks with empty names (spurious
  outputs from some LLMs) and removes their orphaned `ToolResult` counterparts (`orka-llm`)
- Worker session-lock contention now uses exponential backoff (base 1 s, cap 60 s) and
  sends the message to the DLQ after 30 failed retries instead of re-enqueuing indefinitely
  with a fixed 1 s delay (`orka-worker`)

## [1.2.0] - 2026-04-01

### Added

- `orka_core::Error::Checkpoint` and `Error::Experience` typed variants (with
  `source` + `context` fields) and their constructor helpers `Error::checkpoint()`,
  `Error::checkpoint_msg()`, `Error::experience()`, `Error::experience_msg()` —
  replaces the previous `Error::Other(e.to_string())` fallback across checkpoint and
  experience call-sites

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
- **`orka-git`** — `create_git_skills` now returns `Vec<Arc<dyn Skill>>` (was
  `Vec<Box<dyn Skill>>`) and `orka_core::Error` (was `GitError`), aligning with the
  rest of the workspace skill factories
- **`orka-cli` onboard** — `handle_store_secret` and `handle_ask_user` extracted as
  dedicated methods on `OnboardSession`; `to_orka_config` gated behind `#[cfg(test)]`
- `ConversationStatus` default now uses `#[derive(Default)]` + `#[default]` attribute
  instead of a manual `impl Default`
- `RedisConversationStore::list_messages` pagination uses `map_or` (clippy `map_unwrap_or`)

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
