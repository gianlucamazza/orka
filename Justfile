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

# Build release binaries, then install as systemd service (requires sudo)
install: build-release
    sudo ./scripts/install.sh

# Uninstall orka-server and orka CLI (requires sudo)
uninstall:
    sudo ./scripts/install.sh --uninstall

# Release a new version (usage: just release patch|minor|major)
release level:
    cargo release {{level}} --execute

# Dry-run a release to preview what would happen
release-dry level:
    cargo release {{level}}

# Validate config and show version
config-check:
    cargo run -p orka-cli -- config check

# Migrate config to current version (dry-run)
config-migrate-dry:
    cargo run -p orka-cli -- config migrate --dry-run

# Migrate config to current version
config-migrate:
    cargo run -p orka-cli -- config migrate

# Generate PNG icons from SVG (requires librsvg)
icons:
    #!/usr/bin/env bash
    for size in 16 24 32 48 64 128 256 512; do
        mkdir -p assets/icons/hicolor/${size}x${size}/apps
        rsvg-convert -w $size -h $size assets/icons/orka.svg \
            -o assets/icons/hicolor/${size}x${size}/apps/orka.png
    done
    mkdir -p assets/icons/hicolor/scalable/apps
    cp assets/icons/orka.svg assets/icons/hicolor/scalable/apps/orka.svg

# Install icons and desktop entries to ~/.local/share (user-local)
install-desktop:
    #!/usr/bin/env bash
    set -euo pipefail
    ICON_DIR="$HOME/.local/share/icons/hicolor"
    DESKTOP_DIR="$HOME/.local/share/applications"
    for size_dir in assets/icons/hicolor/*/; do
        size=$(basename "$size_dir")
        [ -f "$size_dir/apps/orka.png" ] && \
            install -Dm644 "$size_dir/apps/orka.png" "${ICON_DIR}/${size}/apps/orka.png"
    done
    install -Dm644 assets/icons/hicolor/scalable/apps/orka.svg "${ICON_DIR}/scalable/apps/orka.svg"
    install -Dm644 assets/desktop/orka.desktop "${DESKTOP_DIR}/orka.desktop"
    install -Dm644 assets/desktop/orka-server.desktop "${DESKTOP_DIR}/orka-server.desktop"
    gtk-update-icon-cache -f -t "${ICON_DIR}" 2>/dev/null || true
    echo "Desktop entries and icons installed to ~/.local/share/"

# Generate sudoers NOPASSWD file for configured commands (does NOT install it)
setup-sudoers:
    cargo run -p orka-cli -- sudo check
    @echo ""
    @echo "Sudoers template: deploy/orka.sudoers"
    @echo "To install: sudo install -m 0440 deploy/orka.sudoers /etc/sudoers.d/orka"

# Regenerate CHANGELOG.md from git history
changelog:
    git-cliff --output CHANGELOG.md
