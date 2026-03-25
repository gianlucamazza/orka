#!/usr/bin/env bash
set -euo pipefail

PREFIX="\033[1;36m[orka]\033[0m"
info() { echo -e "${PREFIX} $*"; }
warn() { echo -e "${PREFIX} \033[1;33m$*\033[0m"; }
error() { echo -e "${PREFIX} \033[1;31m$*\033[0m"; }
ok() { echo -e "${PREFIX} \033[1;32m$*\033[0m"; }

PKG_MGR=""
REDIS_SERVICE=""
PACKAGES=()
REQUIRED_RUST=""

detect_distro() {
	if command -v pacman &>/dev/null; then
		PKG_MGR="pacman"
		REDIS_SERVICE="valkey.service"
		PACKAGES=(base-devel pkg-config openssl valkey just curl git)
	elif command -v apt-get &>/dev/null; then
		PKG_MGR="apt"
		REDIS_SERVICE="redis-server.service"
		# Note: `just` is not in standard apt repos; it is installed via cargo after rustup (see below).
		PACKAGES=(build-essential pkg-config libssl-dev redis-server curl git)
	elif command -v dnf &>/dev/null; then
		PKG_MGR="dnf"
		REDIS_SERVICE="redis.service"
		PACKAGES=(gcc gcc-c++ make pkgconf-pkg-config openssl-devel redis just curl git)
	else
		error "Unsupported distribution. Expected pacman, apt-get, or dnf."
		exit 1
	fi
}

detect_required_rust() {
	REQUIRED_RUST=$(grep -oP 'rust-version\s*=\s*"\K[^"]+' "$REPO_ROOT/Cargo.toml" | head -n1 || true)
	if [[ -z "$REQUIRED_RUST" ]]; then
		error "Unable to determine required Rust version from Cargo.toml"
		exit 1
	fi
}

is_pkg_installed() {
	local pkg="$1"
	case "$PKG_MGR" in
	pacman) pacman -Q "$pkg" &>/dev/null ;;
	apt) dpkg -s "$pkg" &>/dev/null ;;
	dnf) rpm -q "$pkg" &>/dev/null ;;
	esac
}

install_packages() {
	local missing=()

	for pkg in "${PACKAGES[@]}"; do
		if ! is_pkg_installed "$pkg"; then
			missing+=("$pkg")
		fi
	done

	if ((${#missing[@]} == 0)); then
		info "System packages up to date ✓"
		return
	fi

	info "Installing missing packages: ${missing[*]}"
	case "$PKG_MGR" in
	pacman)
		sudo pacman -S --needed --noconfirm "${missing[@]}"
		;;
	apt)
		sudo apt-get update
		sudo apt-get install -y "${missing[@]}"
		;;
	dnf)
		sudo dnf install -y "${missing[@]}"
		;;
	esac
}

version_lt() {
	local lhs="$1"
	local rhs="$2"
	[[ "$(printf '%s\n%s\n' "$lhs" "$rhs" | sort -V | head -n1)" != "$rhs" ]]
}

ensure_rust() {
	if ! command -v rustup &>/dev/null; then
		error "rustup not found. Orka expects a rustup-managed toolchain."
		echo "Install rustup, then rerun setup:"
		echo "  curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"
		exit 1
	fi

	if ! command -v cargo &>/dev/null || ! command -v rustc &>/dev/null; then
		error "cargo/rustc not found in PATH. Ensure your rustup environment is loaded."
		echo "For example:"
		echo "  source \"\$HOME/.cargo/env\""
		exit 1
	fi

	local rust_ver
	rust_ver=$(rustc --version | awk '{print $2}')
	if version_lt "$rust_ver" "$REQUIRED_RUST"; then
		error "Rust >= $REQUIRED_RUST required (found $rust_ver)."
		echo "Update your toolchain, then rerun setup:"
		echo "  rustup update stable"
		echo "  rustup override set stable"
		exit 1
	fi

	info "Rust $rust_ver ✓"
}

start_redis() {
	if ! command -v systemctl &>/dev/null; then
		warn "systemctl not found; start Redis/Valkey manually for local development."
		return
	fi

	if ! systemctl is-active --quiet "$REDIS_SERVICE"; then
		info "Starting $REDIS_SERVICE..."
		sudo systemctl enable --now "$REDIS_SERVICE"
	else
		info "$REDIS_SERVICE running ✓"
	fi
}

setup_qdrant() {
	if ! command -v systemctl &>/dev/null; then
		warn "systemctl not found; start Qdrant manually if knowledge.enabled = true."
		return
	fi

	if command -v qdrant &>/dev/null; then
		if [[ ! -f /etc/systemd/system/qdrant.service ]] && ! systemctl list-unit-files qdrant.service &>/dev/null; then
			info "Creating qdrant.service (not shipped by the installed package)..."
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
		if [[ "$PKG_MGR" == "pacman" ]]; then
			warn "Install: yay -S qdrant-bin"
		fi
		warn "Or run:  docker run -d -p 6334:6334 qdrant/qdrant:v1.14.0"
		warn "Or set [knowledge] enabled = false in orka.toml to skip."
	fi
}

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
detect_distro
detect_required_rust
install_packages
ensure_rust

# On Debian/Ubuntu, `just` is not in apt — install via cargo if not already present.
if [[ "$PKG_MGR" == "apt" ]] && ! command -v just &>/dev/null; then
	info "Installing 'just' via cargo (not available in apt)..."
	cargo install just
fi

if [[ ! -f "$REPO_ROOT/.env" ]]; then
	cp "$REPO_ROOT/.env.example" "$REPO_ROOT/.env"
	info "Created .env from .env.example"
else
	info ".env already exists ✓"
fi

start_redis
setup_qdrant

info "Running cargo check --workspace..."
(cd "$REPO_ROOT" && cargo check --workspace)

echo ""
ok "Dev environment ready!"
info "Run the server with: just run"
info "Run tests with:      just test"
