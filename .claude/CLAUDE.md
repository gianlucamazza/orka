# Orka — Project Context for Claude Code

## What is Orka

Orka is an AI agent orchestration framework written in Rust. It manages LLM-powered agents with message routing, skill execution, memory, and multi-platform adapters (Telegram, Discord, Slack, WhatsApp).

## Workspace Structure

Cargo workspace with ~34 crates under `crates/`, plus `orka-server/` (main binary), `crates/orka-cli/` (CLI binary), and `sdk/orka-plugin-sdk/` (WASM plugin SDK).

## Common Commands

```bash
just ci          # Run all checks (check + test + clippy + fmt-check)
just test        # Run unit tests
just clippy      # Lint
just fmt         # Auto-format
just run         # Start the server
just dev         # Docker Compose with hot-reload

cargo test -p orka-core        # Test a single crate
cargo test -- --ignored        # Integration tests (requires Redis)
```

## Conventions

- **MSRV:** 1.85
- **Error handling:** `thiserror` for library crates, `anyhow` sparingly in binaries
- **Formatting:** `rustfmt` defaults, no `rustfmt.toml`
- **Linting:** `cargo clippy -- -D warnings`
- **Comments and docs:** English
- **License:** MIT OR Apache-2.0

## Infrastructure

- **Redis 7+** required for bus (Redis Streams) and queue (Sorted Sets)
- **Qdrant v1.14+** required for the knowledge/RAG vector store (`orka-knowledge`)
- Integration tests use `testcontainers` (Docker required)
- Production Docker image uses `cargo-chef` for layer caching
- `docker-compose.yml` starts Redis + Qdrant + orka-server together

## Key Subsystems

- **orka-knowledge**: RAG pipeline — document ingestion, embedding, and semantic search via Qdrant
- **orka-experience**: Self-learning loop — trajectory recording, post-task reflection, and offline distillation of principles
- **orka-scheduler**: Cron-based task scheduler backed by Redis Sorted Sets
- **OpenAPI/Swagger**: Available at `http://localhost:8080/docs` when the server is running

## CI

GitHub Actions workflow at `.github/workflows/ci.yml` runs: fmt check, clippy, cargo-audit, cargo-deny, build, unit tests, integration tests with Redis and Qdrant service containers, and commitlint (PR only).
