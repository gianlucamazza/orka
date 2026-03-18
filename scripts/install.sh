#!/usr/bin/env bash
set -euo pipefail

PREFIX="\033[1;36m[orka]\033[0m"
info() { echo -e "${PREFIX} $*"; }
warn() { echo -e "${PREFIX} \033[1;33m$*\033[0m"; }
error() { echo -e "${PREFIX} \033[1;31m$*\033[0m"; }
ok() { echo -e "${PREFIX} \033[1;32m$*\033[0m"; }

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

# ── Detect package manager ──────────────────────────────────────────
detect_pkg_manager() {
	if command -v pacman &>/dev/null; then
		PKG_MGR="pacman"
		PKG_SUDOERS_LINES=(
			"orka ALL=(ALL) NOPASSWD: /usr/bin/pacman -S *"
			"orka ALL=(ALL) NOPASSWD: /usr/bin/pacman -Syu *"
		)
		PKG_ALLOWED_CMDS='"pacman -S", "pacman -Syu"'
	elif command -v apt &>/dev/null; then
		PKG_MGR="apt"
		PKG_SUDOERS_LINES=(
			"orka ALL=(ALL) NOPASSWD: /usr/bin/apt install *"
			"orka ALL=(ALL) NOPASSWD: /usr/bin/apt update"
			"orka ALL=(ALL) NOPASSWD: /usr/bin/apt upgrade *"
		)
		PKG_ALLOWED_CMDS='"apt install", "apt update", "apt upgrade"'
	elif command -v dnf &>/dev/null; then
		PKG_MGR="dnf"
		PKG_SUDOERS_LINES=(
			"orka ALL=(ALL) NOPASSWD: /usr/bin/dnf install *"
			"orka ALL=(ALL) NOPASSWD: /usr/bin/dnf update *"
		)
		PKG_ALLOWED_CMDS='"dnf install", "dnf update"'
	else
		PKG_MGR=""
		PKG_SUDOERS_LINES=()
		PKG_ALLOWED_CMDS=""
	fi
}

detect_pkg_manager

BIN_PATH="/usr/local/bin/orka-server"
CLI_BIN_PATH="/usr/local/bin/orka"
UNIT_DIR="/usr/lib/systemd/system"
SYSUSERS_DIR="/usr/lib/sysusers.d"
TMPFILES_DIR="/usr/lib/tmpfiles.d"
CONFIG_DIR="/etc/orka"
DATA_DIR="/var/lib/orka"
ICON_DIR="/usr/local/share/icons/hicolor"
DESKTOP_DIR="/usr/local/share/applications"

SERVICE_NAME="orka-server"
DROPIN_DIR="/etc/systemd/system/${SERVICE_NAME}.service.d"

# ── Check if sudo is enabled in config ──────────────────────────────
# Parses the TOML config for [os.sudo] enabled = true.
check_sudo_enabled() {
	local cfg="${1:-${CONFIG_DIR}/orka.toml}"
	[[ -f "$cfg" ]] || return 1
	# Look for enabled = true under [os.sudo]
	awk '
		/^\[os\.sudo\]/ { in_section=1; next }
		/^\[/           { in_section=0 }
		in_section && /^enabled\s*=\s*true/ { found=1; exit }
		END { exit !found }
	' "$cfg"
}

# ── Root check ───────────────────────────────────────────────────────
if ((EUID != 0)); then
	error "This script must be run as root (or via sudo)."
	exit 1
fi

# ── Uninstall ────────────────────────────────────────────────────────
uninstall() {
	info "Stopping ${SERVICE_NAME}..."
	systemctl stop "${SERVICE_NAME}.service" 2>/dev/null || true
	systemctl disable "${SERVICE_NAME}.service" 2>/dev/null || true

	info "Removing installed files..."
	rm -f "${BIN_PATH}"
	rm -f "${CLI_BIN_PATH}"
	rm -f "${UNIT_DIR}/${SERVICE_NAME}.service"
	rm -f "${SYSUSERS_DIR}/${SERVICE_NAME}.conf"
	rm -f "${TMPFILES_DIR}/${SERVICE_NAME}.conf"
	rm -f /etc/sudoers.d/orka
	rm -rf "${DROPIN_DIR}"

	info "Removing icons and desktop entries..."
	for size in 16 24 32 48 64 128 256 512; do
		rm -f "${ICON_DIR}/${size}x${size}/apps/orka.png"
	done
	rm -f "${ICON_DIR}/scalable/apps/orka.svg"
	rm -f "${DESKTOP_DIR}/orka.desktop"
	rm -f "${DESKTOP_DIR}/orka-server.desktop"
	gtk-update-icon-cache -f -t "${ICON_DIR}" 2>/dev/null || true

	info "Removing shell completions..."
	rm -f /usr/share/bash-completion/completions/orka
	rm -f /usr/share/zsh/site-functions/_orka
	rm -f /usr/share/fish/vendor_completions.d/orka.fish

	systemctl daemon-reload

	echo ""
	ok "Uninstalled orka-server and orka CLI."
	warn "Config (${CONFIG_DIR}) and data (${DATA_DIR}) preserved."
	warn "Remove manually if no longer needed."
}

FORCE=false
for arg in "$@"; do
	case "$arg" in
	--uninstall)
		uninstall
		exit 0
		;;
	--force | --skip-stale-check)
		FORCE=true
		;;
	*)
		error "Unknown option: $arg"
		exit 1
		;;
	esac
done

# ── Check pre-built binaries ─────────────────────────────────────────
SERVER_BIN="$REPO_ROOT/target/release/orka-server"
CLI_BIN="$REPO_ROOT/target/release/orka"

if [[ ! -f "$SERVER_BIN" || ! -f "$CLI_BIN" ]]; then
	error "Release binaries not found at target/release/"
	error "Build them first:  cargo build --release  (or: just build-release)"
	exit 1
fi

# ── Staleness check ─────────────────────────────────────────────────
# Warn if binaries are older than any source file, which can cause
# hard-to-debug crashes from stale intermediate builds.
NEWEST_SRC=$(find "$REPO_ROOT" \( -name target -o -name .git \) -prune -o \
	-type f \( -name '*.rs' -o -name '*.toml' \) -printf '%T@\n' | sort -rn | head -1)
OLDEST_BIN=$(stat -c '%Y' "$SERVER_BIN" "$CLI_BIN" | sort -n | head -1)

if [[ -n "$NEWEST_SRC" ]] && ((${OLDEST_BIN%%.*} < ${NEWEST_SRC%%.*})); then
	warn "Release binaries are older than source files!"
	warn "Run 'cargo build --release' before installing to avoid stale-binary issues."
	if [[ "$FORCE" == true ]]; then
		info "Continuing anyway (--force)."
	elif [[ ! -t 0 ]]; then
		error "Non-interactive shell detected and binaries are stale."
		error "Rebuild first, or pass --force to skip this check."
		exit 1
	else
		read -r -p "$(echo -e "${PREFIX}") Continue anyway? [y/N] " REPLY
		if [[ ! "$REPLY" =~ ^[Yy]$ ]]; then
			info "Aborted. Rebuild with: cargo build --release"
			exit 1
		fi
	fi
fi

# ── Stop service if running ──────────────────────────────────────────
WAS_RUNNING=false
if systemctl is-active --quiet "${SERVICE_NAME}.service"; then
	info "Stopping running ${SERVICE_NAME}..."
	systemctl stop "${SERVICE_NAME}.service"
	WAS_RUNNING=true
fi

# ── Install binary ───────────────────────────────────────────────────
info "Installing binary → ${BIN_PATH}"
install -Dm755 "$REPO_ROOT/target/release/orka-server" "$BIN_PATH"

info "Installing CLI binary → ${CLI_BIN_PATH}"
install -Dm755 "$REPO_ROOT/target/release/orka" "$CLI_BIN_PATH"

# ── Install systemd files ────────────────────────────────────────────
info "Installing systemd unit → ${UNIT_DIR}/${SERVICE_NAME}.service"
install -Dm644 "$REPO_ROOT/deploy/${SERVICE_NAME}.service" "${UNIT_DIR}/${SERVICE_NAME}.service"

info "Installing sysusers config → ${SYSUSERS_DIR}/${SERVICE_NAME}.conf"
install -Dm644 "$REPO_ROOT/deploy/${SERVICE_NAME}.sysusers" "${SYSUSERS_DIR}/${SERVICE_NAME}.conf"

info "Installing tmpfiles config → ${TMPFILES_DIR}/${SERVICE_NAME}.conf"
install -Dm644 "$REPO_ROOT/deploy/${SERVICE_NAME}.tmpfiles" "${TMPFILES_DIR}/${SERVICE_NAME}.conf"

# ── Create user and directories ──────────────────────────────────────
info "Creating system user and directories..."
systemd-sysusers "${SYSUSERS_DIR}/${SERVICE_NAME}.conf"
systemd-tmpfiles --create "${TMPFILES_DIR}/${SERVICE_NAME}.conf"

# ── Config files ─────────────────────────────────────────────────────
if [[ ! -f "${CONFIG_DIR}/orka.toml" ]]; then
	info "Installing default config → ${CONFIG_DIR}/orka.toml"
	install -Dm644 "$REPO_ROOT/orka.toml" "${CONFIG_DIR}/orka.toml"
	# Adjust workspace_dir for production layout
	sed -i 's|^workspace_dir = ".*"|workspace_dir = "/var/lib/orka/workspaces"|' "${CONFIG_DIR}/orka.toml"
	# Inject detected package manager commands into allowed_commands
	if [[ -n "$PKG_ALLOWED_CMDS" ]]; then
		sed -i "s|allowed_commands = \[\"systemctl restart\", \"systemctl stop\", \"systemctl start\"\]|allowed_commands = [\"systemctl restart\", \"systemctl stop\", \"systemctl start\", ${PKG_ALLOWED_CMDS}]|" "${CONFIG_DIR}/orka.toml"
		info "Added ${PKG_MGR} commands to sudo allowed_commands."
	fi
else
	info "Config ${CONFIG_DIR}/orka.toml already exists, skipping."
	info "Checking config migration..."
	if "${CLI_BIN_PATH}" config migrate --config "${CONFIG_DIR}/orka.toml"; then
		ok "Config up to date."
	else
		warn "Config migration failed. Check manually: orka config check --config ${CONFIG_DIR}/orka.toml"
	fi
fi

# ── Install default workspace files ───────────────────────────────
WORKSPACE_DIR="${DATA_DIR}/workspaces"
for ws_file in SOUL.md TOOLS.md; do
	ws_src="$REPO_ROOT/workspaces/${ws_file}"
	ws_dst="${WORKSPACE_DIR}/${ws_file}"
	if [[ ! -f "$ws_dst" ]] && [[ -f "$ws_src" ]]; then
		info "Installing default workspace file → ${ws_dst}"
		install -Dm644 "$ws_src" "$ws_dst"
		chown orka:orka "$ws_dst"
	fi
done

if [[ ! -f "${CONFIG_DIR}/orka.env" ]]; then
	info "Creating empty env file → ${CONFIG_DIR}/orka.env"
	install -m 0640 /dev/null "${CONFIG_DIR}/orka.env"
	chown root:orka "${CONFIG_DIR}/orka.env"
else
	info "Env file ${CONFIG_DIR}/orka.env already exists, skipping."
fi

# ── Install sudoers ──────────────────────────────────────────────────
SUDOERS_SRC="$REPO_ROOT/deploy/orka.sudoers"
SUDOERS_DST="/etc/sudoers.d/orka"

if [[ -f "$SUDOERS_DST" ]]; then
	info "Sudoers file ${SUDOERS_DST} already exists, skipping."
else
	# Build sudoers from base template + detected package manager
	SUDOERS_TMP=$(mktemp)
	cp "$SUDOERS_SRC" "$SUDOERS_TMP"

	if [[ -n "$PKG_MGR" ]]; then
		info "Detected package manager: ${PKG_MGR}"
		echo "" >>"$SUDOERS_TMP"
		echo "# Package manager (${PKG_MGR}) — auto-detected by install script" >>"$SUDOERS_TMP"
		for line in "${PKG_SUDOERS_LINES[@]}"; do
			echo "$line" >>"$SUDOERS_TMP"
		done
	else
		warn "No supported package manager detected — sudoers will only include systemctl."
	fi

	info "Validating generated sudoers..."
	if visudo -cf "$SUDOERS_TMP"; then
		info "Installing sudoers → ${SUDOERS_DST}"
		install -m 0440 "$SUDOERS_TMP" "$SUDOERS_DST"
		ok "Sudoers file installed."
	else
		warn "Sudoers validation failed — skipping installation."
		warn "Review ${SUDOERS_SRC} and install manually."
	fi
	rm -f "$SUDOERS_TMP"
fi

# ── Install systemd drop-in for sudo ────────────────────────────────
# When sudo is enabled, NoNewPrivileges must be relaxed or sudo will fail.
if check_sudo_enabled "${CONFIG_DIR}/orka.toml"; then
	info "sudo enabled in config — installing systemd drop-in to relax NoNewPrivileges"
	mkdir -p "$DROPIN_DIR"
	install -Dm644 "$REPO_ROOT/deploy/orka-server-sudo.conf" "${DROPIN_DIR}/sudo.conf"
	ok "Drop-in installed → ${DROPIN_DIR}/sudo.conf"
else
	# Remove stale drop-in if sudo was previously enabled
	if [[ -f "${DROPIN_DIR}/sudo.conf" ]]; then
		info "sudo disabled in config — removing systemd drop-in"
		rm -f "${DROPIN_DIR}/sudo.conf"
		rmdir --ignore-fail-on-non-empty "$DROPIN_DIR" 2>/dev/null || true
	fi
fi

# ── Install shell completions ────────────────────────────────────────
info "Generating shell completions..."

BASH_COMP_DIR="/usr/share/bash-completion/completions"
ZSH_COMP_DIR="/usr/share/zsh/site-functions"
FISH_COMP_DIR="/usr/share/fish/vendor_completions.d"

if [[ -d "$BASH_COMP_DIR" ]]; then
	"${CLI_BIN_PATH}" completions bash >"${BASH_COMP_DIR}/orka"
	ok "Bash completions → ${BASH_COMP_DIR}/orka"
fi

if [[ -d "$ZSH_COMP_DIR" ]]; then
	"${CLI_BIN_PATH}" completions zsh >"${ZSH_COMP_DIR}/_orka"
	ok "Zsh completions → ${ZSH_COMP_DIR}/_orka"
fi

if [[ -d "$FISH_COMP_DIR" ]]; then
	"${CLI_BIN_PATH}" completions fish >"${FISH_COMP_DIR}/orka.fish"
	ok "Fish completions → ${FISH_COMP_DIR}/orka.fish"
fi

# ── Install icons ────────────────────────────────────────────────────
info "Installing icons..."
for size_dir in "$REPO_ROOT/assets/icons/hicolor"/*/; do
	size=$(basename "$size_dir")
	[ -f "$size_dir/apps/orka.png" ] &&
		install -Dm644 "$size_dir/apps/orka.png" "${ICON_DIR}/${size}/apps/orka.png"
done
install -Dm644 "$REPO_ROOT/assets/icons/hicolor/scalable/apps/orka.svg" \
	"${ICON_DIR}/scalable/apps/orka.svg"

# ── Install desktop entries ──────────────────────────────────────────
info "Installing desktop entries..."
install -Dm644 "$REPO_ROOT/assets/desktop/orka.desktop" "${DESKTOP_DIR}/orka.desktop"
install -Dm644 "$REPO_ROOT/assets/desktop/orka-server.desktop" "${DESKTOP_DIR}/orka-server.desktop"

# Update icon cache
gtk-update-icon-cache -f -t "${ICON_DIR}" 2>/dev/null || true

# ── Reload systemd ───────────────────────────────────────────────────
systemctl daemon-reload

# ── Restart service if it was running ────────────────────────────────
if [[ "$WAS_RUNNING" == true ]]; then
	info "Restarting ${SERVICE_NAME}..."
	systemctl start "${SERVICE_NAME}.service"
	ok "${SERVICE_NAME} restarted."
fi

# ── Done ─────────────────────────────────────────────────────────────
echo ""
ok "orka-server and orka CLI installed successfully!"
echo ""
info "Enable and start the service:"
echo "  systemctl enable --now ${SERVICE_NAME}"
echo ""
info "View logs:"
echo "  journalctl -u ${SERVICE_NAME} -f"
