# Changelog

All notable changes to Orka will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/), and this project adheres to [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [1.0.0] - 2026-03-18

### Added

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
