#!/usr/bin/env bash
set -euo pipefail

PREFIX="\033[1;36m[orka]\033[0m"
info() { echo -e "${PREFIX} $*"; }
warn() { echo -e "${PREFIX} \033[1;33m$*\033[0m"; }
error() { echo -e "${PREFIX} \033[1;31m$*\033[0m"; }
ok() { echo -e "${PREFIX} \033[1;32m$*\033[0m"; }

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

BIN_PATH="/usr/local/bin/orka-server"
CLI_BIN_PATH="/usr/local/bin/orka"
UNIT_DIR="/usr/lib/systemd/system"
SYSUSERS_DIR="/usr/lib/sysusers.d"
TMPFILES_DIR="/usr/lib/tmpfiles.d"
CONFIG_DIR="/etc/orka"
DATA_DIR="/var/lib/orka"

SERVICE_NAME="orka-server"

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

	systemctl daemon-reload

	echo ""
	ok "Uninstalled orka-server and orka CLI."
	warn "Config (${CONFIG_DIR}) and data (${DATA_DIR}) preserved."
	warn "Remove manually if no longer needed."
}

if [[ "${1:-}" == "--uninstall" ]]; then
	uninstall
	exit 0
fi

# ── Build ────────────────────────────────────────────────────────────
info "Building orka-server and orka CLI (release)..."
(cd "$REPO_ROOT" && cargo build --release --bin orka-server --bin orka)

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
else
	info "Config ${CONFIG_DIR}/orka.toml already exists, skipping."
fi

if [[ ! -f "${CONFIG_DIR}/orka.env" ]]; then
	info "Creating empty env file → ${CONFIG_DIR}/orka.env"
	install -m 0640 /dev/null "${CONFIG_DIR}/orka.env"
	chown root:orka "${CONFIG_DIR}/orka.env"
else
	info "Env file ${CONFIG_DIR}/orka.env already exists, skipping."
fi

# ── Reload systemd ───────────────────────────────────────────────────
systemctl daemon-reload

# ── Done ─────────────────────────────────────────────────────────────
echo ""
ok "orka-server and orka CLI installed successfully!"
echo ""
info "Enable and start the service:"
echo "  systemctl enable --now ${SERVICE_NAME}"
echo ""
info "View logs:"
echo "  journalctl -u ${SERVICE_NAME} -f"
