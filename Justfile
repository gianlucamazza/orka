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

# Start infra (Redis)
infra:
    docker-compose up -d

# Stop infra
infra-down:
    docker-compose down

# Run server with infra
up: infra run

# Run the server
run:
    cargo run -p orka-server

# Run the server with debug logging
run-debug:
    RUST_LOG=debug cargo run -p orka-server

# Build release
build-release:
    cargo build --workspace --release

# Build Docker image with OCI labels
docker-build:
    docker build \
        --build-arg BUILD_DATE=$(date -u +'%Y-%m-%dT%H:%M:%SZ') \
        --build-arg VCS_REF=$(git rev-parse --short HEAD) \
        --build-arg VERSION=$(git describe --tags --always --dirty 2>/dev/null || echo dev) \
        -t orka-server:dev .

# Clean build artifacts
clean:
    cargo clean

# Run a specific crate's tests
test-crate crate:
    cargo test -p {{crate}}

# Watch and recompile on changes (requires cargo-watch)
watch:
    cargo watch -x 'check --workspace'

# Start dev environment with hot-reload
dev:
    docker compose up --build --watch

# Rebuild dev image from scratch
dev-rebuild:
    docker compose build --no-cache orka-server

# Bootstrap dev environment (Arch Linux)
setup:
    ./scripts/setup-dev.sh

# Install orka-server as a systemd service (requires sudo)
install:
    sudo ./scripts/install.sh

# Uninstall orka-server systemd service (requires sudo)
uninstall:
    sudo ./scripts/install.sh --uninstall
