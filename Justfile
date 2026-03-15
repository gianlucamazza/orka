# Orka development commands

# Run all checks (CI equivalent)
ci: check test clippy fmt-check

# Compile all crates
check:
    cargo check --workspace

# Run all tests
test:
    cargo test --workspace

# Run clippy lints
clippy:
    cargo clippy --workspace -- -D warnings

# Check formatting
fmt-check:
    cargo fmt --all -- --check

# Auto-format all code
fmt:
    cargo fmt --all

# Run the server
run:
    cargo run -p orka-server

# Run the server with debug logging
run-debug:
    RUST_LOG=debug cargo run -p orka-server

# Build release
build-release:
    cargo build --workspace --release

# Clean build artifacts
clean:
    cargo clean

# Run a specific crate's tests
test-crate crate:
    cargo test -p {{crate}}

# Watch and recompile on changes (requires cargo-watch)
watch:
    cargo watch -x 'check --workspace'
