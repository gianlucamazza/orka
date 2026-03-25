# Orka — Project Context for Claude Code

## What is Orka

Orka is an AI agent orchestration framework written in Rust. It manages LLM-powered agents with message routing, skill execution, memory, and multi-platform adapters (Telegram, Discord, Slack, WhatsApp).

## Workspace Structure

Cargo workspace with 38 members: 35 under `crates/` (including `orka-server` binary), plus 3 under `sdk/` (WASM plugin SDKs and example plugin).

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

- **MSRV:** 1.91
- **Error handling:** `thiserror` for library crates, `anyhow` sparingly in binaries
- **Formatting:** `.rustfmt.toml` present with unstable options — run `cargo +nightly fmt --all` (stable options also work with `cargo fmt --all`)
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

## Common Commands (extended)

```bash
just infra           # Start Redis + Qdrant only (no server)
just infra-down      # Stop infrastructure
just test-crate orka-core  # Test a single crate
just test-e2e        # End-to-end tests
just watch           # Watch mode (cargo check on change)
just build-release   # Build release binary
just config-check    # Validate orka.toml
just config-migrate  # Migrate orka.toml to current version
just install         # Install system service (systemd)
just uninstall       # Remove system service
just docs            # Generate and open rustdoc
just setup           # First-time setup (deps + Redis + build verify)
just msrv            # Check MSRV compliance (1.91)
```

## CI

GitHub Actions workflows:
- `.github/workflows/ci.yml` — fmt check, clippy, cargo-audit, cargo-deny, build, unit tests, integration tests (Redis + Qdrant), MSRV check, coverage, commitlint (PR only)
- `.github/workflows/packaging.yml` — Debian and Fedora package linting
- `.github/workflows/release.yml` — release automation
- `.github/workflows/typos.yml` — typo checking across the codebase
