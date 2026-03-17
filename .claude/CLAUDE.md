# Orka — Project Context for Claude Code

## What is Orka

Orka is an AI agent orchestration framework written in Rust. It manages LLM-powered agents with message routing, skill execution, memory, and multi-platform adapters (Telegram, Discord, Slack, WhatsApp).

## Workspace Structure

Cargo workspace with ~30 crates under `crates/`, plus `orka-server/` (main binary), `crates/orka-cli/` (CLI binary), and `sdk/orka-plugin-sdk/` (WASM plugin SDK).

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

- **MSRV:** 1.75
- **Error handling:** `thiserror` for library crates, `anyhow` sparingly in binaries
- **Formatting:** `rustfmt` defaults, no `rustfmt.toml`
- **Linting:** `cargo clippy -- -D warnings`
- **Comments and docs:** English
- **License:** MIT OR Apache-2.0

## Infrastructure

- **Redis 7+** required for bus (Redis Streams) and queue (Sorted Sets)
- Integration tests use `testcontainers` (Docker required)
- Production Docker image uses `cargo-chef` for layer caching

## CI

GitHub Actions workflow at `.github/workflows/ci.yml` runs: fmt check, clippy, cargo-audit, cargo-deny, build, unit tests, and integration tests with a Redis service container.
