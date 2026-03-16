#!/usr/bin/env bash
set -euo pipefail

PREFIX="\033[1;36m[orka]\033[0m"
info() { echo -e "${PREFIX} $*"; }
warn() { echo -e "${PREFIX} \033[1;33m$*\033[0m"; }
error() { echo -e "${PREFIX} \033[1;31m$*\033[0m"; }
ok() { echo -e "${PREFIX} \033[1;32m$*\033[0m"; }

# ── Check OS ─────────────────────────────────────────────────────────
if [[ ! -f /etc/arch-release ]]; then
	error "This script targets Arch Linux. Detected a different distribution."
	exit 1
fi

# ── Check Rust toolchain ─────────────────────────────────────────────
if ! command -v rustup &>/dev/null || ! command -v cargo &>/dev/null; then
	error "rustup/cargo not found. Install Rust first:"
	echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
	exit 1
fi

RUST_VER=$(rustc --version | grep -oP '\d+\.\d+')
RUST_MAJOR=$(echo "$RUST_VER" | cut -d. -f1)
RUST_MINOR=$(echo "$RUST_VER" | cut -d. -f2)
if ((RUST_MAJOR < 1 || (RUST_MAJOR == 1 && RUST_MINOR < 75))); then
	error "Rust >= 1.75 required (found $RUST_VER). Run: rustup update stable"
	exit 1
fi
info "Rust $RUST_VER ✓"

# ── Install system packages ──────────────────────────────────────────
PACKAGES=(base-devel pkg-config openssl valkey just)
MISSING=()

for pkg in "${PACKAGES[@]}"; do
	if ! pacman -Q "$pkg" &>/dev/null; then
		MISSING+=("$pkg")
	fi
done

if ((${#MISSING[@]} > 0)); then
	info "Installing missing packages: ${MISSING[*]}"
	sudo pacman -S --needed --noconfirm "${MISSING[@]}"
else
	info "System packages up to date ✓"
fi

# ── Copy .env.example → .env ─────────────────────────────────────────
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

if [[ ! -f "$REPO_ROOT/.env" ]]; then
	cp "$REPO_ROOT/.env.example" "$REPO_ROOT/.env"
	info "Created .env from .env.example"
else
	info ".env already exists ✓"
fi

# ── Start Redis ──────────────────────────────────────────────────────
if ! systemctl is-active --quiet redis.service; then
	info "Starting redis.service..."
	sudo systemctl enable --now redis.service
else
	info "Redis running ✓"
fi

# ── Qdrant (optional, for knowledge/RAG) ──────────────────────────────
if command -v qdrant &>/dev/null; then
	if [[ ! -f /etc/systemd/system/qdrant.service ]] && ! systemctl list-unit-files qdrant.service &>/dev/null; then
		info "Creating qdrant.service (not shipped by qdrant-bin)..."
		sudo tee /etc/systemd/system/qdrant.service >/dev/null <<-'UNIT'
			[Unit]
			Description=Qdrant Vector Search Engine
			After=network.target

			[Service]
			Type=simple
			ExecStart=/usr/bin/qdrant --config-path /etc/qdrant/config.yaml
			WorkingDirectory=/var/lib/qdrant
			Restart=on-failure
			RestartSec=5

			[Install]
			WantedBy=multi-user.target
		UNIT
		sudo systemctl daemon-reload
	fi
	if ! systemctl is-active --quiet qdrant.service; then
		info "Starting qdrant.service..."
		sudo systemctl enable --now qdrant.service
	else
		info "Qdrant running ✓"
	fi
else
	warn "Qdrant not found (needed if knowledge.enabled = true in orka.toml)."
	warn "Install: yay -S qdrant-bin"
	warn "Or run:  docker run -d -p 6334:6334 qdrant/qdrant:v1.14.0"
	warn "Or set [knowledge] enabled = false in orka.toml to skip."
fi

# ── Build verification ───────────────────────────────────────────────
info "Running cargo check --workspace..."
(cd "$REPO_ROOT" && cargo check --workspace)

# ── Done ─────────────────────────────────────────────────────────────
echo ""
ok "Dev environment ready!"
info "Run the server with: just run"
info "Run tests with:      just test"
