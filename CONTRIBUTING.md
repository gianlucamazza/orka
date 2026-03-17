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
just ci           # Full check suite: fmt + clippy + test
just test         # Unit tests only
just clippy       # Lint (warnings are errors)
just fmt          # Auto-format

cargo test -p orka-core        # Test a single crate
cargo test -- --include-ignored # Integration tests (requires Redis)
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

## Architecture

Orka is organized as a Cargo workspace with ~30 crates. Each crate has a single responsibility:

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

See the [README](README.md) for the full project structure.

## Reporting Issues

Use [GitHub Issues](https://github.com/gianlucamazza/orka/issues) for bug reports and feature requests. For security vulnerabilities, see [SECURITY.md](SECURITY.md).

## License

By contributing, you agree that your contributions will be licensed under both the [MIT License](LICENSE-MIT) and [Apache License 2.0](LICENSE-APACHE), at the choice of the user.
