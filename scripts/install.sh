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

detect_systemd_dirs() {
	local unit_dir=""
	local sysusers_dir=""
	local tmpfiles_dir=""

	if command -v pkg-config &>/dev/null && pkg-config --exists systemd; then
		unit_dir="$(pkg-config --variable=systemdsystemunitdir systemd 2>/dev/null || true)"
		sysusers_dir="$(pkg-config --variable=sysusersdir systemd 2>/dev/null || true)"
		tmpfiles_dir="$(pkg-config --variable=tmpfilesdir systemd 2>/dev/null || true)"
	fi

	UNIT_DIR="${unit_dir:-/usr/lib/systemd/system}"
	SYSUSERS_DIR="${sysusers_dir:-/usr/lib/sysusers.d}"
	TMPFILES_DIR="${tmpfiles_dir:-/usr/lib/tmpfiles.d}"
}

detect_systemd_dirs

# Fixed FHS paths — do not change with --prefix
CONFIG_DIR="/etc/orka"
DATA_DIR="/var/lib/orka"

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
# Parses the TOML config for [os.sudo] allowed = true.
check_sudo_enabled() {
	local cfg="${1:-${CONFIG_DIR}/orka.toml}"
	[[ -f "$cfg" ]] || return 1
	# Look for allowed = true under [os.sudo]
	awk '
		/^\[os\.sudo\]/ { in_section=1; next }
		/^\[/           { in_section=0 }
		in_section && /^allowed\s*=\s*true/ { found=1; exit }
		END { exit !found }
	' "$cfg"
}

get_config_value() {
	local cfg="$1"
	local section="$2"
	local key="$3"

	awk -v target_section="$section" -v target_key="$key" '
		$0 == "[" target_section "]" { in_section=1; next }
		/^\[/                    { in_section=0 }
		in_section && $0 ~ "^[[:space:]]*" target_key "[[:space:]]*=" {
			line=$0
			sub(/^[^=]*=[[:space:]]*/, "", line)
			gsub(/^"/, "", line)
			gsub(/"$/, "", line)
			print line
			exit
		}
	' "$cfg"
}

config_section_enabled() {
	local cfg="$1"
	local section="$2"
	[[ -f "$cfg" ]] || return 1

	awk -v target_section="$section" '
		$0 == "[" target_section "]" { in_section=1; next }
		/^\[/                    { in_section=0 }
		in_section && /^[[:space:]]*enabled[[:space:]]*=[[:space:]]*true/ { found=1; exit }
		END { exit !found }
	' "$cfg"
}

service_user_home() {
	getent passwd orka | awk -F: '{ print $6 }'
}

sudo_user_home() {
	[[ -n "${SUDO_USER:-}" ]] || return 1
	getent passwd "${SUDO_USER}" | awk -F: '{ print $6 }'
}

append_pkg_sudo_commands() {
	local cfg="$1"
	local tmp
	tmp=$(mktemp)

	awk -v pkg_cmds="$PKG_ALLOWED_CMDS" '
		function trim(s) {
			sub(/^[[:space:]]+/, "", s)
			sub(/[[:space:]]+$/, "", s)
			return s
		}
		function ensure_section_defaults() {
			if (!seen_allowed_commands) {
				print "allowed_commands = [" pkg_cmds "]"
				seen_allowed_commands=1
			}
		}
		BEGIN {
			in_sudo=0
			seen_sudo=0
			seen_allowed_commands=0
		}
		/^\[os\.sudo\]/ {
			seen_sudo=1
			in_sudo=1
			print
			next
		}
		/^\[/ {
			if (in_sudo) {
				ensure_section_defaults()
			}
			in_sudo=0
			print
			next
		}
		in_sudo && /^allowed_commands[[:space:]]*=/ {
			seen_allowed_commands=1
			line=$0
			start=index(line, "[")
			end=index(line, "]")
			if (start == 0 || end == 0 || end <= start) {
				print $0
				next
			}
			body=substr(line, start + 1, end - start - 1)
			if (trim(body) == "") {
				print "allowed_commands = [" pkg_cmds "]"
			} else {
				print substr(line, 1, start) body ", " pkg_cmds "]"
			}
			next
		}
		{
			print
		}
		END {
			if (!seen_sudo) {
				print ""
				print "[os.sudo]"
				print "allowed = false"
				print "allowed_commands = [" pkg_cmds "]"
				print "password_required = true"
			} else if (in_sudo) {
				ensure_section_defaults()
			}
		}
	' "$cfg" >"$tmp"

	mv "$tmp" "$cfg"
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
	warn "The installed systemd unit will be patched to use ${INSTALL_PREFIX}/bin/orka-server."
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

# ── Shared helpers for binaries under /home ──────────────────────────
grant_home_binary_acls() {
	local label="$1"
	local cmd_path="$2"

	command -v setfacl &>/dev/null || {
		warn "setfacl not found — install acl package for ${label} binary access"
		return 0
	}

	local real_path
	real_path=$(readlink -f "$cmd_path" 2>/dev/null) || return 0

	info "Setting ACLs for ${label} binary: ${cmd_path}"

	setfacl -m u:orka:rx "$real_path" 2>/dev/null &&
		ok "  u:orka:rx → ${real_path}"

	local dir
	dir=$(dirname "$real_path")
	while [[ "$dir" != "/" && "$dir" == /home* ]]; do
		setfacl -m u:orka:x "$dir" 2>/dev/null
		dir=$(dirname "$dir")
	done

	if [[ "$cmd_path" != "$real_path" ]]; then
		dir=$(dirname "$cmd_path")
		while [[ "$dir" != "/" && "$dir" == /home* ]]; do
			setfacl -m u:orka:x "$dir" 2>/dev/null
			dir=$(dirname "$dir")
		done
	fi

	ok "  Traversal ACLs set on path chain"
}

extract_home_mcp_commands() {
	local cfg="${1:-${CONFIG_DIR}/orka.toml}"
	[[ -f "$cfg" ]] || return 0

	awk '
		/^\[\[mcp\.servers\]\]/ { in_mcp=1; next }
		/^\[/                   { in_mcp=0 }
		in_mcp && /^command\s*=/ {
			gsub(/.*=\s*"/, ""); gsub(/".*/, "");
			if (/^\/home/) print
		}
	' "$cfg"
}

extract_home_coding_provider_commands() {
	local cfg="${1:-${CONFIG_DIR}/orka.toml}"
	[[ -f "$cfg" ]] || return 0

	awk '
		/^\[os\.coding\.providers\.(claude_code|codex|opencode)\]/ {
			section=$0
			gsub(/^\[|\]$/, "", section)
			next
		}
		/^\[/ { section="" }
		section != "" && /^executable_path\s*=/ {
			path=$0
			gsub(/.*=\s*"/, "", path)
			gsub(/".*/, "", path)
			if (path ~ /^\/home/) {
				print section "|" path
			}
		}
	' "$cfg"
}

# ── Set ACLs for MCP binaries under /home ────────────────────────────
setup_mcp_acls() {
	local cfg="${1:-${CONFIG_DIR}/orka.toml}"
	local cmds
	cmds=$(extract_home_mcp_commands "$cfg")
	[[ -z "$cmds" ]] && return 0

	while IFS= read -r cmd_path; do
		grant_home_binary_acls "MCP" "$cmd_path"
	done <<<"$cmds"
}

setup_coding_provider_acls() {
	local cfg="${1:-${CONFIG_DIR}/orka.toml}"
	local cmds
	cmds=$(extract_home_coding_provider_commands "$cfg")
	[[ -z "$cmds" ]] && return 0

	while IFS='|' read -r section cmd_path; do
		[[ -n "$cmd_path" ]] || continue
		grant_home_binary_acls "${section}" "$cmd_path"
	done <<<"$cmds"
}

detect_user_codex_installation() {
	local user_home
	user_home=$(sudo_user_home) || return 1

	local codex_path=""
	if [[ -n "${SUDO_USER:-}" ]]; then
		codex_path=$(runuser -u "${SUDO_USER}" -- bash -lc 'command -v codex 2>/dev/null || true' 2>/dev/null || true)
	fi

	if [[ -z "$codex_path" && -d "${user_home}/.local/share/fnm/node-versions" ]]; then
		codex_path=$(find "${user_home}/.local/share/fnm/node-versions" -path '*/installation/bin/codex' 2>/dev/null | sort | tail -n1 || true)
	fi

	[[ -n "$codex_path" ]] || return 1

	local resolved_codex node_path js_path
	resolved_codex=$(readlink -f "$codex_path" 2>/dev/null) || return 1
	js_path="$resolved_codex"

	# Try node as sibling of codex (fnm / nvm layout), then fall back to
	# the user's PATH (system node or other manager).
	node_path=$(readlink -f "$(dirname "$codex_path")/node" 2>/dev/null) || true
	if [[ ! -x "${node_path:-}" && -n "${SUDO_USER:-}" ]]; then
		node_path=$(runuser -u "${SUDO_USER}" -- bash -lc 'command -v node 2>/dev/null || true' 2>/dev/null || true)
		[[ -n "$node_path" ]] && node_path=$(readlink -f "$node_path" 2>/dev/null) || true
	fi

	[[ -x "${node_path:-}" ]] || return 1
	[[ -f "$js_path" ]] || return 1

	printf '%s|%s\n' "$node_path" "$js_path"
}

install_codex_wrapper() {
	local cfg="${1:-${CONFIG_DIR}/orka.toml}"
	local codex_target
	codex_target=$(get_config_value "$cfg" "os.coding.providers.codex" "executable_path")
	[[ -n "$codex_target" ]] || codex_target="/usr/local/bin/codex"

	if ! config_section_enabled "$cfg" "os.coding.providers.codex"; then
		return 0
	fi

	local runtime
	runtime=$(detect_user_codex_installation) || {
		warn "Codex provider enabled but no user installation was found for ${SUDO_USER:-current user}"
		return 0
	}

	local node_path js_path
	node_path=${runtime%%|*}
	js_path=${runtime#*|}

	info "Installing Codex wrapper → ${codex_target}"
	install -d "$(dirname "$codex_target")"
	cat >"${codex_target}" <<EOF
#!/usr/bin/env bash
exec "${node_path}" "${js_path}" "\$@"
EOF
	chmod 0755 "${codex_target}"

	grant_home_binary_acls "Codex runtime" "$node_path"
	grant_home_binary_acls "Codex launcher" "$js_path"
}

provision_codex_service_home() {
	local cfg="${1:-${CONFIG_DIR}/orka.toml}"
	config_section_enabled "$cfg" "os.coding.providers.codex" || return 0

	local user_home service_home
	user_home=$(sudo_user_home) || {
		warn "Codex provider enabled but SUDO_USER is unavailable; skipping Codex credential bootstrap"
		return 0
	}
	service_home=$(service_user_home)
	[[ -n "$service_home" ]] || return 0

	local src_dir="${user_home}/.codex"
	local dst_dir="${service_home}/.codex"
	[[ -f "${src_dir}/auth.json" ]] || {
		warn "Codex provider enabled but ${src_dir}/auth.json was not found"
		return 0
	}

	info "Provisioning Codex state for service user → ${dst_dir}"
	install -d -o orka -g orka -m 0700 "${dst_dir}"

	if [[ -f "${src_dir}/auth.json" ]]; then
		install -o orka -g orka -m 0600 "${src_dir}/auth.json" "${dst_dir}/auth.json"
	fi
	if [[ -f "${src_dir}/config.toml" ]]; then
		install -o orka -g orka -m 0600 "${src_dir}/config.toml" "${dst_dir}/config.toml"
	fi
	if [[ -f "${src_dir}/version.json" ]]; then
		install -o orka -g orka -m 0644 "${src_dir}/version.json" "${dst_dir}/version.json"
	fi
}

# ── Install ───────────────────────────────────────────────────────────
do_install() {
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

	trap 'rm -f "$SUDOERS_TMP"' RETURN

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
		} | awk 'BEGIN { max = "" } { if (max == "" || $1 > max) max = $1 } END { if (max != "") print max }'
	)
	NEWEST_BIN=$(stat -c '%Y' "$SERVER_BIN" "$CLI_BIN" | awk 'NR == 1 || $1 > max { max = $1 } END { print max }')

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
	sed -i "s|@BINDIR@|${INSTALL_PREFIX}/bin|g" "${UNIT_DIR}/${SERVICE_NAME}.service"

	info "Installing sysusers config → ${SYSUSERS_DIR}/${SERVICE_NAME}.conf"
	install -Dm644 "$REPO_ROOT/deploy/${SERVICE_NAME}.sysusers" "${SYSUSERS_DIR}/${SERVICE_NAME}.conf"

	info "Installing tmpfiles config → ${TMPFILES_DIR}/${SERVICE_NAME}.conf"
	install -Dm644 "$REPO_ROOT/deploy/${SERVICE_NAME}.tmpfiles" "${TMPFILES_DIR}/${SERVICE_NAME}.conf"

	# ── Create user and directories ───────────────────────────────────
	info "Creating system user and directories..."
	systemd-sysusers "${SYSUSERS_DIR}/${SERVICE_NAME}.conf"
	systemd-tmpfiles --create "${TMPFILES_DIR}/${SERVICE_NAME}.conf"
	# Ensure orka can read journal logs via journalctl (needed by journal_read skill).
	run_cmd usermod -aG systemd-journal orka 2>/dev/null || true
	info "Added orka to systemd-journal group for journal access."

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
		# Inject detected package manager commands into os.sudo.allowed_commands.
		if [[ -n "$PKG_ALLOWED_CMDS" ]]; then
			append_pkg_sudo_commands "${CONFIG_DIR}/orka.toml"
			info "Added ${PKG_MGR} commands to sudo allowed_commands."
		fi
	else
		info "Config ${CONFIG_DIR}/orka.toml already exists, skipping."
		info "Checking config migration..."
		if "${CLI_BIN_PATH}" config migrate --config "${CONFIG_DIR}/orka.toml"; then
			ok "Config migration/rewrite completed."
			if "${CLI_BIN_PATH}" config check --config "${CONFIG_DIR}/orka.toml"; then
				ok "Config validation passed."
			else
				error "Config validation failed. Refusing to restart ${SERVICE_NAME} with an invalid config."
				error "Check manually: orka config check --config ${CONFIG_DIR}/orka.toml"
				exit 1
			fi
		else
			error "Config migration failed. Refusing to restart ${SERVICE_NAME}."
			error "Check manually: orka config check --config ${CONFIG_DIR}/orka.toml"
			exit 1
		fi
	fi

	# ── Install/upgrade default workspace files ───────────────────────
	# On fresh install: copy unconditionally.
	# On upgrade: compare the `version` field in the YAML frontmatter and
	# overwrite (with a dated backup) only when the repo version is newer.
	local ws_src ws_dst src_ver dst_ver
	for ws_file in SOUL.md TOOLS.md; do
		ws_src="$REPO_ROOT/workspaces/${ws_file}"
		ws_dst="${WORKSPACE_DIR}/${ws_file}"
		[[ -f "$ws_src" ]] || continue
		if [[ ! -f "$ws_dst" ]]; then
			info "Installing default workspace file → ${ws_dst}"
			install -Dm644 "$ws_src" "$ws_dst"
			chown orka:orka "$ws_dst"
		else
			# Extract version string from frontmatter (e.g. version: "0.2")
			src_ver=$(sed -n '/^---$/,/^---$/{/^version:[[:space:]]*"/{s/.*"\(.*\)"/\1/p;q}}' "$ws_src")
			dst_ver=$(sed -n '/^---$/,/^---$/{/^version:[[:space:]]*"/{s/.*"\(.*\)"/\1/p;q}}' "$ws_dst")
			if [[ -n "$src_ver" && "$src_ver" != "$dst_ver" ]]; then
				info "Upgrading ${ws_file}: ${dst_ver:-unversioned} → ${src_ver}"
				run_cmd cp "$ws_dst" "${ws_dst}.bak.$(date +%Y%m%d)"
				run_cmd install -Dm644 "$ws_src" "$ws_dst"
				run_cmd chown orka:orka "$ws_dst"
				ok "Backed up and upgraded ${ws_file} to version ${src_ver}"
			fi
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

	# ── Set ACLs for MCP binaries under /home ────────────────────────
	install_codex_wrapper "${CONFIG_DIR}/orka.toml"
	provision_codex_service_home "${CONFIG_DIR}/orka.toml"
	setup_mcp_acls "${CONFIG_DIR}/orka.toml"
	setup_coding_provider_acls "${CONFIG_DIR}/orka.toml"

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
		if command -v setfacl &>/dev/null; then
			if [[ -n "${SUDO_USER:-}" && -d "/home/${SUDO_USER}" ]]; then
				echo ""
				info "Granting orka read+traverse access to /home/${SUDO_USER} via ACL..."
				setfacl -m u:orka:rx "/home/${SUDO_USER}"
				ok "ACL set: u:orka:rx on /home/${SUDO_USER}"
			else
				echo ""
				info "To grant orka access to a user's home directory:"
				echo "  sudo setfacl -m u:orka:rx /home/<username>"
			fi
		else
			echo ""
			info "To grant orka access to a user's home directory:"
			echo "  sudo setfacl -m u:orka:rx /home/<username>"
			warn "  (install acl package first: setfacl not found)"
		fi
	fi
}

# ── Dispatch ─────────────────────────────────────────────────────────
case "$ACTION" in
uninstall) uninstall ;;
*) do_install ;;
esac
