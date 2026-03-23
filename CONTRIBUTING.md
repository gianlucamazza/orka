# Contributing to Orka

Thank you for your interest in contributing to Orka!

## Getting Started

### Prerequisites

- Rust 1.85+ (MSRV)
- Redis 7+ (for integration tests)
- Docker (for `testcontainers`-based integration tests)
- [just](https://github.com/casey/just) command runner

### Setup

```bash
git clone https://github.com/gianlucamazza/orka.git
cd orka

# Arch Linux: automated setup (installs deps, starts Redis, verifies build)
just setup

# Or manually:
cargo build --workspace
```

### Running Checks

```bash
just ci           # Full check suite: check + test + clippy + fmt-check
just test         # Unit tests only
just clippy       # Lint (warnings are errors)
just fmt          # Auto-format

cargo test -p orka-core        # Test a single crate
cargo test -- --ignored        # Integration tests (requires Redis)
```

## Development Workflow

1. Fork the repository
2. Create a feature branch from `main`
3. Make your changes — keep PRs focused on a single concern
4. Add tests for new functionality
5. Run `just ci` to verify everything passes
6. Open a pull request with a clear description

### Commit Messages

We use [Conventional Commits](https://www.conventionalcommits.org/). CI enforces this via [commitlint](https://commitlint.js.org/).

```
feat: add new adapter for Matrix
fix: correct session expiry in Redis store
docs: update README with new CLI commands
refactor: simplify LLM client initialization
test: add coverage for priority queue DLQ
chore: bump dependencies
```

### PR Guidelines

- Keep PRs small and focused
- Include tests for new code paths
- Update documentation if the public API changes
- Ensure `just ci` passes before requesting review

## Code Style

- Follow standard Rust conventions (`rustfmt` defaults, no `rustfmt.toml`)
- Lint with `cargo clippy -- -D warnings`
- Use `thiserror` for error types in library crates, `anyhow` sparingly in binaries
- Add tests for new functionality in `#[cfg(test)] mod tests` blocks
- Comments and documentation in English

## Rust Best Practices (2026)

Orka uses Rust 1.93+ with Edition 2024. Follow these modern patterns:

### Prefer `let else` for Early Returns

```rust
// ❌ Avoid
if let Some(value) = option {
    // logic
} else {
    return Err(Error::NotFound);
}

// ✅ Prefer
let Some(value) = option else {
    return Err(Error::NotFound);
};
// logic without extra indentation
```

### Use `std::sync::LazyLock` for Static Initialization

```rust
// ❌ Avoid
use once_cell::sync::Lazy;
static CONFIG: Lazy<String> = Lazy::new(|| load_config());

// ✅ Prefer
use std::sync::LazyLock;
static CONFIG: LazyLock<String> = LazyLock::new(|| load_config());
```

### Async Patterns

```rust
// Use native async traits where possible (Rust 1.75+)
pub trait MyService {
    async fn process(&self) -> Result<T>;
}

// For trait objects, use `async_trait` or return `impl Future`
pub trait ServiceObject {
    fn process(&self) -> impl Future<Output = Result<T>> + Send;
}
```

### Error Handling

- Use `?` operator with `thiserror` for structured errors
- Use `anyhow` only in binaries, not libraries
- Prefer `Result<T, E>` over panics

### Testing

```rust
#[tokio::test]
async fn test_async_function() {
    // Async tests are native, no special setup needed
}
```

## Architecture

Orka is organized as a Cargo workspace with ~34 crates. Each crate has a single responsibility:

- **orka-core**: Shared types, traits, and error definitions
- **orka-bus**: Message bus abstraction (Redis Streams)
- **orka-queue**: Priority queue (Redis Sorted Sets)
- **orka-worker**: Worker pool and message handlers
- **orka-gateway**: Inbound message routing
- **orka-llm**: LLM provider integrations (Anthropic, OpenAI)
- **orka-skills**: Skill registry and execution
- **orka-adapter-\***: Platform adapters (Telegram, Discord, Slack, WhatsApp)
- **orka-os**: Linux OS integration skills
- **orka-web**: Web search and page reading skills
- **orka-knowledge**: RAG/vector store skills
- **orka-observe**: Observability (Prometheus metrics, Redis/OTel event sinks)
- **orka-mcp**: Model Context Protocol server
- **orka-a2a**: Agent-to-Agent protocol
- **orka-guardrails**: Input/output validation and content filtering
- **orka-circuit-breaker**: Circuit breaker pattern for external services
- **orka-sandbox**: Sandboxed code execution (process + WASM)
- **orka-secrets**: Secret management (AES-256-GCM)
- **orka-auth**: JWT and API key authentication
- **orka-http**: HTTP request skill with SSRF protection
- **orka-workspace**: Workspace loader and hot-reload watcher
- **orka-scheduler**: Cron-based task scheduler
- **orka-experience**: Self-learning loop (trajectory recording, reflection, distillation)
- **orka-agent**: Agent orchestration and routing
- **orka-wasm**: WASM runtime utilities

For a deeper dive into each subsystem and how they interact, see [docs/architecture.md](docs/architecture.md).

See the [README](README.md) for the full project structure.

## Reporting Issues

Use [GitHub Issues](https://github.com/gianlucamazza/orka/issues) for bug reports and feature requests. For security vulnerabilities, see [SECURITY.md](SECURITY.md).

## License

By contributing, you agree that your contributions will be licensed under both the [MIT License](LICENSE-MIT) and [Apache License 2.0](LICENSE-APACHE), at the choice of the user.
