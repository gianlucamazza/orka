#!/usr/bin/env bash
set -euo pipefail
umask 022

PREFIX="\033[1;36m[orka]\033[0m"
info() { echo -e "${PREFIX} $*"; }
warn() { echo -e "${PREFIX} \033[1;33m$*\033[0m"; }
error() { echo -e "${PREFIX} \033[1;31m$*\033[0m"; }
ok() { echo -e "${PREFIX} \033[1;32m$*\033[0m"; }

# ── Usage ─────────────────────────────────────────────────────────────
usage() {
	cat <<EOF
Usage: $(basename "$0") [OPTIONS]

Install orka-server and orka CLI as a systemd service.

OPTIONS:
  --profile PROFILE   Cargo build profile to use (default: release)
  --prefix PREFIX     Installation prefix (default: /usr/local)
  --uninstall         Remove installed files
  --purge             With --uninstall: also remove config, data, and system user
  --dry-run           Print what would be done without making changes
  --yes, -y           Non-interactive: skip confirmation prompts
  --force             Skip stale binary check
  --help, -h          Show this help message

EXAMPLES:
  sudo $0                             # Install release binaries
  sudo $0 --profile debug             # Install debug binaries
  sudo $0 --prefix /opt/orka          # Install to /opt/orka/bin/
  sudo $0 --uninstall --yes           # Uninstall without prompting
  sudo $0 --dry-run                   # Preview install actions
EOF
}

# ── Dry-run helpers ───────────────────────────────────────────────────
run_cmd() {
	if [[ "${DRY_RUN:-false}" == true ]]; then
		echo -e "${PREFIX} \033[2m[dry-run]\033[0m $*"
	else
		"$@"
	fi
}

safe_rm() {
	for target in "$@"; do
		if [[ -e "$target" || -L "$target" ]]; then
			info "Removing: $target"
			run_cmd rm -rf -- "$target"
		fi
	done
}

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

# Fixed FHS paths — do not change with --prefix
CONFIG_DIR="/etc/orka"
DATA_DIR="/var/lib/orka"
UNIT_DIR="/usr/lib/systemd/system"
SYSUSERS_DIR="/usr/lib/sysusers.d"
TMPFILES_DIR="/usr/lib/tmpfiles.d"

SERVICE_NAME="orka-server"
DROPIN_DIR="/etc/systemd/system/${SERVICE_NAME}.service.d"

# ── Check if /home paths are in allowed_paths ────────────────────────
check_home_access_needed() {
	local cfg="${1:-${CONFIG_DIR}/orka.toml}"
	[[ -f "$cfg" ]] || return 0 # no config → compiled default includes /home
	# If allowed_paths is not set, the compiled default includes /home
	if ! grep -qE '^\s*allowed_paths\s*=' "$cfg"; then
		return 0
	fi
	grep -qE '^\s*allowed_paths\s*=.*"/home' "$cfg"
}

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

# ── Arg defaults ─────────────────────────────────────────────────────
ACTION=""
PURGE=false
DRY_RUN=false
YES=false
FORCE=false
PROFILE="release"
INSTALL_PREFIX="/usr/local"

# ── Pre-scan for --help (before root check) ──────────────────────────
for arg in "$@"; do
	[[ "$arg" == "--help" || "$arg" == "-h" ]] && {
		usage
		exit 0
	}
done

# ── Root check ───────────────────────────────────────────────────────
if ((EUID != 0)); then
	error "This script must be run as root (or via sudo)."
	exit 1
fi

# ── Arg parsing ──────────────────────────────────────────────────────
while [[ $# -gt 0 ]]; do
	case "$1" in
	--uninstall) ACTION="uninstall" ;;
	--purge) PURGE=true ;;
	--dry-run) DRY_RUN=true ;;
	--yes | -y) YES=true ;;
	--force | --skip-stale-check) FORCE=true ;;
	--profile)
		shift
		[[ $# -eq 0 ]] && {
			error "--profile requires a value"
			exit 1
		}
		PROFILE="$1"
		;;
	--profile=*) PROFILE="${1#--profile=}" ;;
	--prefix)
		shift
		[[ $# -eq 0 ]] && {
			error "--prefix requires a value"
			exit 1
		}
		INSTALL_PREFIX="$1"
		;;
	--prefix=*) INSTALL_PREFIX="${1#--prefix=}" ;;
	--help | -h)
		usage
		exit 0
		;;
	*)
		error "Unknown option: $1"
		exit 1
		;;
	esac
	shift
done

# ── Derive install paths from prefix ─────────────────────────────────
BIN_PATH="${INSTALL_PREFIX}/bin/orka-server"
CLI_BIN_PATH="${INSTALL_PREFIX}/bin/orka"
ICON_DIR="${INSTALL_PREFIX}/share/icons/hicolor"
DESKTOP_DIR="${INSTALL_PREFIX}/share/applications"

# ── Warn if non-standard prefix ──────────────────────────────────────
if [[ "$INSTALL_PREFIX" != "/usr/local" ]]; then
	warn "Non-standard prefix: ${INSTALL_PREFIX}"
	warn "The systemd service file hardcodes ExecStart=/usr/local/bin/orka-server"
	warn "You will need to update it manually after installation."
fi

# ── Uninstall ────────────────────────────────────────────────────────
uninstall() {
	local files=(
		"${BIN_PATH}"
		"${CLI_BIN_PATH}"
		"${UNIT_DIR}/${SERVICE_NAME}.service"
		"${SYSUSERS_DIR}/${SERVICE_NAME}.conf"
		"${TMPFILES_DIR}/${SERVICE_NAME}.conf"
		"/etc/sudoers.d/orka"
		"${DROPIN_DIR}"
	)
	local icons=()
	for size in 16 24 32 48 64 128 256 512; do
		icons+=("${ICON_DIR}/${size}x${size}/apps/orka.png")
	done
	icons+=(
		"${ICON_DIR}/scalable/apps/orka.svg"
		"${DESKTOP_DIR}/orka.desktop"
		"${DESKTOP_DIR}/orka-server.desktop"
	)
	local completions=(
		"/usr/share/bash-completion/completions/orka"
		"/usr/share/zsh/site-functions/_orka"
		"/usr/share/fish/vendor_completions.d/orka.fish"
	)

	# Pre-removal summary
	echo ""
	info "The following will be removed:"
	for f in "${files[@]}" "${icons[@]}" "${completions[@]}"; do
		[[ -e "$f" || -L "$f" ]] && echo "  $f"
	done
	if [[ "${PURGE:-false}" == true ]]; then
		[[ -d "${CONFIG_DIR}" ]] && echo "  ${CONFIG_DIR}  (purge)"
		[[ -d "${DATA_DIR}" ]] && echo "  ${DATA_DIR}  (purge)"
		id orka &>/dev/null && echo "  system user: orka  (purge)"
	fi
	echo ""

	# Confirmation
	if [[ "${DRY_RUN:-false}" != true ]]; then
		if [[ "${YES:-false}" == true ]]; then
			: # skip prompt
		elif [[ ! -t 0 ]]; then
			error "Non-interactive shell: pass --yes to confirm uninstall."
			exit 1
		else
			read -r -p "$(echo -e "${PREFIX}") Proceed with uninstall? [y/N] " REPLY
			if [[ ! "$REPLY" =~ ^[Yy]$ ]]; then
				info "Aborted."
				exit 1
			fi
		fi
	fi

	# Stop and disable service
	info "Stopping ${SERVICE_NAME}..."
	if systemctl is-active --quiet "${SERVICE_NAME}.service" 2>/dev/null; then
		run_cmd systemctl stop "${SERVICE_NAME}.service"
	fi
	if systemctl is-enabled --quiet "${SERVICE_NAME}.service" 2>/dev/null; then
		run_cmd systemctl disable "${SERVICE_NAME}.service"
	fi
	run_cmd systemctl reset-failed "${SERVICE_NAME}.service" 2>/dev/null || true

	# Remove files
	info "Removing installed files..."
	safe_rm "${files[@]}"

	# Remove icons and desktop entries
	info "Removing icons and desktop entries..."
	safe_rm "${icons[@]}"
	if command -v update-desktop-database &>/dev/null; then
		run_cmd update-desktop-database "${DESKTOP_DIR}" 2>/dev/null || true
	fi

	# Remove shell completions
	info "Removing shell completions..."
	safe_rm "${completions[@]}"

	run_cmd systemctl daemon-reload
	command -v gtk-update-icon-cache &>/dev/null &&
		run_cmd gtk-update-icon-cache -f -t "${ICON_DIR}" 2>/dev/null || true

	# Purge config, data, and system user
	if [[ "${PURGE:-false}" == true ]]; then
		info "Purging config, data, and system user..."
		safe_rm "${CONFIG_DIR}" "${DATA_DIR}"
		if id orka &>/dev/null; then
			info "Removing system user: orka"
			run_cmd userdel orka 2>/dev/null || true
		fi
		if getent group orka &>/dev/null; then
			info "Removing system group: orka"
			run_cmd groupdel orka 2>/dev/null || true
		fi
	fi

	echo ""
	if [[ "${DRY_RUN:-false}" == true ]]; then
		ok "Dry run complete — nothing was changed."
	else
		ok "Uninstalled orka-server and orka CLI."
		if [[ "${PURGE:-false}" != true ]]; then
			warn "Config (${CONFIG_DIR}) and data (${DATA_DIR}) preserved."
			warn "Use --purge to remove them."
		fi
		warn "Journal logs are NOT removed automatically."
		warn "To clean up:  journalctl --vacuum-time=1s -u ${SERVICE_NAME}"
	fi
}

# ── Install ───────────────────────────────────────────────────────────
install() {
	local WAS_RUNNING=false
	local SERVER_BIN="$REPO_ROOT/target/${PROFILE}/orka-server"
	local CLI_BIN="$REPO_ROOT/target/${PROFILE}/orka"
	local SUDOERS_TMP=""
	local WORKSPACE_DIR="${DATA_DIR}/workspaces"
	local NEWEST_SRC NEWEST_BIN
	local BASH_COMP_DIR="/usr/share/bash-completion/completions"
	local ZSH_COMP_DIR="/usr/share/zsh/site-functions"
	local FISH_COMP_DIR="/usr/share/fish/vendor_completions.d"
	local version git_rev

	trap 'rm -f "$SUDOERS_TMP"' EXIT

	# ── Check pre-built binaries ──────────────────────────────────────
	if [[ ! -f "$SERVER_BIN" || ! -f "$CLI_BIN" ]]; then
		error "${PROFILE^} binaries not found at target/${PROFILE}/"
		if [[ "$PROFILE" == "release" ]]; then
			error "Build them first:  cargo build --release  (or: just build-release)"
		else
			error "Build them first:  cargo build --profile ${PROFILE}"
		fi
		exit 1
	fi

	# ── Staleness check ───────────────────────────────────────────────
	# Check only source directories to avoid noise from generated files.
	NEWEST_SRC=$(
		{
			for src_dir in "$REPO_ROOT/crates" "$REPO_ROOT/orka-server" "$REPO_ROOT/sdk"; do
				[[ -d "$src_dir" ]] && find "$src_dir" \
					-type f \( -name '*.rs' -o -name 'Cargo.toml' \) -printf '%T@\n' 2>/dev/null
			done
			find "$REPO_ROOT" -maxdepth 1 \( -name 'Cargo.toml' -o -name 'Cargo.lock' \) \
				-printf '%T@\n' 2>/dev/null
		} | sort -rn | head -1
	)
	NEWEST_BIN=$(stat -c '%Y' "$SERVER_BIN" "$CLI_BIN" | sort -rn | head -1)

	if [[ -n "$NEWEST_SRC" ]] && ((${NEWEST_BIN%%.*} < ${NEWEST_SRC%%.*})); then
		warn "${PROFILE^} binaries are older than source files!"
		if [[ "$PROFILE" == "release" ]]; then
			warn "Run 'cargo build --release' before installing to avoid stale-binary issues."
		else
			warn "Run 'cargo build --profile ${PROFILE}' before installing to avoid stale-binary issues."
		fi
		if [[ "$FORCE" == true ]]; then
			info "Continuing anyway (--force)."
		elif [[ ! -t 0 ]]; then
			error "Non-interactive shell detected and binaries are stale."
			error "Rebuild first, or pass --force to skip this check."
			exit 1
		else
			read -r -p "$(echo -e "${PREFIX}") Continue anyway? [y/N] " REPLY
			if [[ ! "$REPLY" =~ ^[Yy]$ ]]; then
				if [[ "$PROFILE" == "release" ]]; then
					info "Aborted. Rebuild with: cargo build --release"
				else
					info "Aborted. Rebuild with: cargo build --profile ${PROFILE}"
				fi
				exit 1
			fi
		fi
	fi

	# ── Stop service if running ───────────────────────────────────────
	if systemctl is-active --quiet "${SERVICE_NAME}.service"; then
		info "Stopping running ${SERVICE_NAME}..."
		systemctl stop "${SERVICE_NAME}.service"
		WAS_RUNNING=true
	fi

	# ── Install binaries ──────────────────────────────────────────────
	info "Installing binary → ${BIN_PATH}"
	install -Dm755 "$SERVER_BIN" "$BIN_PATH"

	info "Installing CLI binary → ${CLI_BIN_PATH}"
	install -Dm755 "$CLI_BIN" "$CLI_BIN_PATH"

	# ── Install systemd files ─────────────────────────────────────────
	info "Installing systemd unit → ${UNIT_DIR}/${SERVICE_NAME}.service"
	install -Dm644 "$REPO_ROOT/deploy/${SERVICE_NAME}.service" "${UNIT_DIR}/${SERVICE_NAME}.service"

	info "Installing sysusers config → ${SYSUSERS_DIR}/${SERVICE_NAME}.conf"
	install -Dm644 "$REPO_ROOT/deploy/${SERVICE_NAME}.sysusers" "${SYSUSERS_DIR}/${SERVICE_NAME}.conf"

	info "Installing tmpfiles config → ${TMPFILES_DIR}/${SERVICE_NAME}.conf"
	install -Dm644 "$REPO_ROOT/deploy/${SERVICE_NAME}.tmpfiles" "${TMPFILES_DIR}/${SERVICE_NAME}.conf"

	# ── Create user and directories ───────────────────────────────────
	info "Creating system user and directories..."
	systemd-sysusers "${SYSUSERS_DIR}/${SERVICE_NAME}.conf"
	systemd-tmpfiles --create "${TMPFILES_DIR}/${SERVICE_NAME}.conf"

	# ── Config files ──────────────────────────────────────────────────
	if [[ ! -f "${CONFIG_DIR}/orka.toml" ]]; then
		info "Installing default config → ${CONFIG_DIR}/orka.toml"
		install -Dm644 "$REPO_ROOT/orka.toml" "${CONFIG_DIR}/orka.toml"
		# Adjust workspace_dir for production layout
		sed -i 's|^workspace_dir = ".*"|workspace_dir = "/var/lib/orka/workspaces"|' "${CONFIG_DIR}/orka.toml"
		# Ensure /var/lib/orka is in allowed_paths so PermissionGuard permits the
		# service's state directory under systemd ProtectSystem=strict.
		if grep -q '^allowed_paths' "${CONFIG_DIR}/orka.toml"; then
			if ! grep -q '"/var/lib/orka"' "${CONFIG_DIR}/orka.toml"; then
				if grep -q '^allowed_paths = \[\]' "${CONFIG_DIR}/orka.toml"; then
					# Empty array — replace whole value to avoid ", " prefix
					sed -i 's|^allowed_paths = \[\]|allowed_paths = ["/var/lib/orka"]|' "${CONFIG_DIR}/orka.toml"
				else
					sed -i 's|^\(allowed_paths = \[.*\)\]|\1, "/var/lib/orka"]|' "${CONFIG_DIR}/orka.toml"
				fi
			fi
		else
			sed -i 's|^#.*allowed_paths = \[.*\]|allowed_paths = ["/home", "/tmp", "/var/lib/orka"]|' "${CONFIG_DIR}/orka.toml"
		fi
		# Inject detected package manager commands into allowed_commands.
		# Uses a flexible regex so it works regardless of what entries are already in the array.
		if [[ -n "$PKG_ALLOWED_CMDS" ]]; then
			if grep -q '^allowed_commands = \[\]' "${CONFIG_DIR}/orka.toml"; then
				# Empty array — replace whole value
				sed -i "s|^allowed_commands = \[\]|allowed_commands = [${PKG_ALLOWED_CMDS}]|" "${CONFIG_DIR}/orka.toml"
			else
				# Non-empty array — append before closing bracket
				sed -i "s|^\(allowed_commands = \[.*\)\]|\1, ${PKG_ALLOWED_CMDS}]|" "${CONFIG_DIR}/orka.toml"
			fi
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

	# ── Install default workspace files ──────────────────────────────
	local ws_src ws_dst
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

	# ── Install sudoers ───────────────────────────────────────────────
	local SUDOERS_SRC="$REPO_ROOT/deploy/orka.sudoers"
	local SUDOERS_DST="/etc/sudoers.d/orka"

	if [[ -f "$SUDOERS_DST" ]]; then
		info "Sudoers file ${SUDOERS_DST} already exists, skipping."
	else
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
	fi

	# ── Install systemd drop-in for sudo ──────────────────────────────
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

	# ── Install systemd drop-in for filesystem access ─────────────────
	# When allowed_paths includes /home, ProtectHome must be relaxed.
	if check_home_access_needed "${CONFIG_DIR}/orka.toml"; then
		info "allowed_paths includes /home — installing fs drop-in to relax ProtectHome"
		mkdir -p "$DROPIN_DIR"
		install -Dm644 "$REPO_ROOT/deploy/orka-server-fs.conf" "${DROPIN_DIR}/fs.conf"
		ok "Drop-in installed → ${DROPIN_DIR}/fs.conf"
	else
		if [[ -f "${DROPIN_DIR}/fs.conf" ]]; then
			info "No /home paths in allowed_paths — removing fs drop-in"
			rm -f "${DROPIN_DIR}/fs.conf"
			rmdir --ignore-fail-on-non-empty "$DROPIN_DIR" 2>/dev/null || true
		fi
	fi

	# ── Install shell completions ─────────────────────────────────────
	info "Generating shell completions..."

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

	# ── Install icons ─────────────────────────────────────────────────
	info "Installing icons..."
	for size_dir in "$REPO_ROOT/assets/icons/hicolor"/*/; do
		local size
		size=$(basename "$size_dir")
		[ -f "$size_dir/apps/orka.png" ] &&
			install -Dm644 "$size_dir/apps/orka.png" "${ICON_DIR}/${size}/apps/orka.png"
	done
	install -Dm644 "$REPO_ROOT/assets/icons/hicolor/scalable/apps/orka.svg" \
		"${ICON_DIR}/scalable/apps/orka.svg"

	# ── Install desktop entries ───────────────────────────────────────
	info "Installing desktop entries..."
	install -Dm644 "$REPO_ROOT/assets/desktop/orka.desktop" "${DESKTOP_DIR}/orka.desktop"
	install -Dm644 "$REPO_ROOT/assets/desktop/orka-server.desktop" "${DESKTOP_DIR}/orka-server.desktop"

	# Update icon cache
	command -v gtk-update-icon-cache &>/dev/null &&
		gtk-update-icon-cache -f -t "${ICON_DIR}" 2>/dev/null || true

	# ── Reload systemd ────────────────────────────────────────────────
	systemctl daemon-reload

	# ── Write install info stamp ──────────────────────────────────────
	version=$(grep -m1 '^version\s*=' "$REPO_ROOT/Cargo.toml" | sed 's/.*= "\(.*\)"/\1/' || echo "unknown")
	git_rev=$(git -C "$REPO_ROOT" describe --tags --always --dirty 2>/dev/null || echo "unknown")
	mkdir -p "${CONFIG_DIR}"
	printf 'version=%s git=%s profile=%s prefix=%s date=%s\n' \
		"$version" "$git_rev" "$PROFILE" "$INSTALL_PREFIX" \
		"$(date -u +%Y-%m-%dT%H:%M:%SZ)" >"${CONFIG_DIR}/.install-info"
	ok "Install info written → ${CONFIG_DIR}/.install-info"

	# ── Restart service if it was running ────────────────────────────
	if [[ "$WAS_RUNNING" == true ]]; then
		info "Restarting ${SERVICE_NAME}..."
		systemctl start "${SERVICE_NAME}.service"
		ok "${SERVICE_NAME} restarted."
	fi

	# ── Done ──────────────────────────────────────────────────────────
	echo ""
	ok "orka-server and orka CLI installed successfully!"
	echo ""
	info "Enable and start the service:"
	echo "  systemctl enable --now ${SERVICE_NAME}"
	echo ""
	info "View logs:"
	echo "  journalctl -u ${SERVICE_NAME} -f"

	if check_home_access_needed "${CONFIG_DIR}/orka.toml"; then
		echo ""
		info "To grant orka access to a user's home directory:"
		echo "  sudo usermod -aG \$(id -gn <username>) orka"
		echo "  chmod g+rx /home/<username>"
	fi
}

# ── Dispatch ─────────────────────────────────────────────────────────
case "$ACTION" in
uninstall) uninstall ;;
*) install ;;
esac
