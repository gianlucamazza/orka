# Contributing to Orka

Thank you for your interest in contributing to Orka!

## Getting Started

### Prerequisites

- Rust 1.75+
- Redis 7+
- Docker (for integration tests)

### Setup

```bash
git clone https://github.com/gianlucamazza/orka.git
cd orka
cargo build --workspace
```

### Running Tests

```bash
# Unit tests
cargo test --workspace

# Including integration tests (requires Redis)
cargo test --workspace -- --include-ignored
```

## Development Workflow

1. Fork the repository
2. Create a feature branch (`git checkout -b feature/my-feature`)
3. Make your changes
4. Run checks:
   ```bash
   cargo fmt --all -- --check
   cargo clippy --workspace --all-targets
   cargo test --workspace
   ```
5. Commit with a clear message
6. Open a pull request

## Code Style

- Follow standard Rust conventions (`rustfmt` defaults)
- Keep functions focused and small
- Use `thiserror` for error types, `anyhow` sparingly
- Add tests for new functionality
- Comments in English

## Architecture

Orka is organized as a Cargo workspace with ~30 crates. Each crate has a single responsibility:

- **orka-core**: Shared types, traits, and error definitions
- **orka-bus**: Message bus abstraction (Redis Streams)
- **orka-queue**: Priority queue (Redis Sorted Sets)
- **orka-worker**: Worker pool and message handlers
- **orka-gateway**: Inbound message routing
- **orka-llm**: LLM provider integrations
- **orka-skills**: Skill registry and execution

See the [README](README.md) for the full project structure.

## Reporting Issues

Use GitHub Issues with the provided templates for bug reports and feature requests.

## License

By contributing, you agree that your contributions will be licensed under the MIT License.
