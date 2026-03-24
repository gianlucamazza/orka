# Orka - Gemini Context

This document provides essential context, workflows, and conventions for working on the **Orka** project. Orka is a self-learning AI agent orchestration platform built in Rust.

## Project Overview

Orka is a modular, event-driven system designed to orchestrate AI agents. It routes messages from various platforms (Telegram, Discord, Slack, etc.) through a priority queue to LLM-powered agents. These agents can execute tools, build knowledge via RAG, and learn from experience.

### Architecture
The system follows a **Modular Monolith** architecture that is **Distributed-Ready**.
- **Event-Driven Core:** All communication occurs via a message bus backed by **Redis Streams**.
- **Agentic Loop:** Agents operate in a loop: Observation -> Thought -> Action (Tool Use) -> Observation.
- **Extensibility:** Supports **WASM** plugins (via Wasmtime), **MCP** (Model Context Protocol), and **Agent-to-Agent** (A2A) communication.
- **Layers:**
    - **Adapters:** Normalize external platform messages into internal `Envelope`s.
    - **Gateway:** Handles deduplication, rate-limiting, and routing to the priority queue.
    - **Worker Pool:** Consumes messages from the queue and executes the agent logic.
    - **Knowledge:** RAG system using **Qdrant**.

### Key Technologies
- **Language:** Rust (MSRV 1.85+)
- **Async Runtime:** `tokio`
- **Web Framework:** `axum`
- **Messaging & State:** `redis` (Streams, Sorted Sets, KV)
- **Vector Database:** `qdrant-client`
- **LLM Integration:** Abstracted providers (Anthropic, OpenAI, Ollama)
- **Sandboxing:** `wasmtime` (WASM Component Model)
- **Observability:** `opentelemetry`, `prometheus`, `tracing`
- **Serialization:** `serde`, `serde_json`

## Environment Setup

### Prerequisites
- **Rust:** 1.85+
- **Redis:** 7+ (Required for messaging and state)
- **Docker:** Optional, useful for running infrastructure (`docker compose`).
- **Tools:** `just` (Command runner - **Highly Recommended**), `bacon` (Background checks).

### Quick Start
The project uses `just` to automate common tasks.

```bash
# Install dependencies, start Redis, and check the build (Arch Linux)
just setup

# Start infrastructure (Redis) only
just infra

# Run the server
just run
```

## Development Workflow

### Building
```bash
# Build all crates
cargo build --workspace

# Build release version
cargo build --workspace --release
```

### Testing
**Unit Tests:**
```bash
# Run all unit tests
just test
# OR
cargo test --workspace
```

**Integration Tests:**
Integration tests require a running Redis instance.
```bash
# Run integration tests (marked with #[ignore])
cargo test --workspace -- --ignored
```

**Continuous Feedback:**
Use `bacon` for instant feedback during development.
```bash
bacon         # Default: cargo check
bacon test    # Run tests
bacon clippy  # Run clippy
```

### Linting & Formatting
```bash
# Linting (Warnings are errors in CI)
just clippy
# OR
cargo clippy --workspace --all-targets -- -D warnings

# Formatting
just fmt
# OR
cargo +nightly fmt --all
```

## Project Structure

The workspace is organized into specialized crates in the `crates/` directory:

| Crate | Responsibility |
| :--- | :--- |
| **`orka-core`** | Core traits (`MessageBus`, `Skill`), types (`Envelope`, `Session`), and errors. |
| **`orka-agent`** | The "Brain": Agent loop, prompt engineering, tool execution. |
| **`orka-bus`** | Message bus implementation (Redis Streams). |
| **`orka-server`** | Main entry point and API server. |
| **`orka-worker`** | Worker pool implementation. |
| **`orka-llm`** | LLM provider abstractions. |
| **`orka-memory`** | Long-term memory and session storage. |
| **`orka-knowledge`** | RAG and Vector DB integration. |
| **`orka-wasm`** | WASM runtime and plugin system. |
| **`orka-adapter-*`** | Platform-specific adapters (Discord, Telegram, Slack, etc.). |

## Conventions & Standards

### Coding Style
- **Rust Edition:** 2024 (where applicable/supported).
- **Error Handling:**
    - Libraries: Use `thiserror` for structured, typed errors.
    - Binaries: Use `anyhow` for flexibility.
    - Prefer `Result<T, E>` over panics.
- **Async:** Use `tokio` idioms. Prefer `let else` for early returns.
- **Static Initialization:** Use `std::sync::LazyLock` instead of `lazy_static` or `once_cell`.

### Git & Commits
- **Conventional Commits:** The project strictly enforces Conventional Commits (e.g., `feat:`, `fix:`, `chore:`).
- **Scope:** PRs should be focused and atomic.

### Configuration
- **File:** `orka.toml`
- **Environment Variables:** `ORKA_*` (e.g., `ORKA_REDIS_URL`).
- **Hot-Reload:** The system supports hot-reloading for `.env` and Workspace files (`SOUL.md`, `TOOLS.md`).

## CLI Tools
- **`orka-cli` (crate):** Management tool.
    - `orka config check`: Validate configuration.
    - `orka config migrate`: Migrate configuration.
- **`just`:**
    - `just ci`: Run full CI suite locally.
    - `just demo`: Record demos using VHS.
    - `just install`: Install systemd services.
