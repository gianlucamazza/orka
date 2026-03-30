#!/usr/bin/env bash
set -euo pipefail

usage() {
	cat <<'EOF'
Usage: deploy-remote-native.sh [OPTIONS]

Upload a native Orka install bundle to a remote host and reinstall the systemd
service using the repository's existing install script.

OPTIONS:
  --bundle-tarball PATH      Bundle tarball created by create-install-bundle.sh
  --host HOST                Remote host
  --user USER                Remote user
  --port PORT                SSH port (default: 22)
  --remote-base-dir DIR      Remote base directory (default: ~/orka-deploy)
  --release-name NAME        Remote release directory name
  --prefix PREFIX            Install prefix passed to install.sh (default: /usr/local)
  --dry-run                  Pass --dry-run to the remote installer
  --help, -h                 Show this help
EOF
}

BUNDLE_TARBALL=""
HOST=""
USER_NAME=""
PORT="22"
REMOTE_BASE_DIR="~/orka-deploy"
RELEASE_NAME=""
INSTALL_PREFIX="/usr/local"
DRY_RUN=false

while [[ $# -gt 0 ]]; do
	case "$1" in
	--bundle-tarball)
		shift
		BUNDLE_TARBALL="${1:?--bundle-tarball requires a value}"
		;;
	--bundle-tarball=*) BUNDLE_TARBALL="${1#--bundle-tarball=}" ;;
	--host)
		shift
		HOST="${1:?--host requires a value}"
		;;
	--host=*) HOST="${1#--host=}" ;;
	--user)
		shift
		USER_NAME="${1:?--user requires a value}"
		;;
	--user=*) USER_NAME="${1#--user=}" ;;
	--port)
		shift
		PORT="${1:?--port requires a value}"
		;;
	--port=*) PORT="${1#--port=}" ;;
	--remote-base-dir)
		shift
		REMOTE_BASE_DIR="${1:?--remote-base-dir requires a value}"
		;;
	--remote-base-dir=*) REMOTE_BASE_DIR="${1#--remote-base-dir=}" ;;
	--release-name)
		shift
		RELEASE_NAME="${1:?--release-name requires a value}"
		;;
	--release-name=*) RELEASE_NAME="${1#--release-name=}" ;;
	--prefix)
		shift
		INSTALL_PREFIX="${1:?--prefix requires a value}"
		;;
	--prefix=*) INSTALL_PREFIX="${1#--prefix=}" ;;
	--dry-run) DRY_RUN=true ;;
	--help | -h)
		usage
		exit 0
		;;
	*)
		echo "Unknown option: $1" >&2
		usage >&2
		exit 1
		;;
	esac
	shift
done

[[ -n "${BUNDLE_TARBALL}" ]] || {
	echo "--bundle-tarball is required" >&2
	exit 1
}
[[ -n "${HOST}" ]] || {
	echo "--host is required" >&2
	exit 1
}
[[ -n "${USER_NAME}" ]] || {
	echo "--user is required" >&2
	exit 1
}
[[ -f "${BUNDLE_TARBALL}" ]] || {
	echo "Bundle tarball not found: ${BUNDLE_TARBALL}" >&2
	exit 1
}

if [[ -z "${RELEASE_NAME}" ]]; then
	RELEASE_NAME="$(basename "${BUNDLE_TARBALL}" .tar.gz)"
fi

REMOTE="${USER_NAME}@${HOST}"
SSH_OPTS=(-p "${PORT}" -o StrictHostKeyChecking=accept-new)
SCP_OPTS=(-P "${PORT}" -o StrictHostKeyChecking=accept-new)
REMOTE_RELEASE_DIR="${REMOTE_BASE_DIR}/releases/${RELEASE_NAME}"
REMOTE_TARBALL="${REMOTE_RELEASE_DIR}/bundle.tar.gz"
REMOTE_EXTRACT_DIR="${REMOTE_RELEASE_DIR}/bundle"

ssh "${SSH_OPTS[@]}" "${REMOTE}" "mkdir -p ${REMOTE_RELEASE_DIR@Q}"
scp "${SCP_OPTS[@]}" "${BUNDLE_TARBALL}" "${REMOTE}:${REMOTE_TARBALL}"

REMOTE_INSTALL_ARGS=(--profile release --force --yes "--prefix=${INSTALL_PREFIX}")
if [[ "${DRY_RUN}" == true ]]; then
	REMOTE_INSTALL_ARGS+=(--dry-run)
fi

ssh "${SSH_OPTS[@]}" "${REMOTE}" bash -s -- \
	"${REMOTE_TARBALL}" \
	"${REMOTE_EXTRACT_DIR}" \
	"${REMOTE_INSTALL_ARGS[@]}" <<'EOF'
set -euo pipefail

remote_tarball="$1"
shift
remote_extract_dir="$1"
shift

rm -rf "${remote_extract_dir}"
mkdir -p "${remote_extract_dir}"
tar -xzf "${remote_tarball}" -C "${remote_extract_dir}" --strip-components=1
cd "${remote_extract_dir}"

sudo ./scripts/install.sh "$@"

if [[ " $* " != *" --dry-run "* ]]; then
	sudo systemctl enable --now orka-server
	sudo systemctl is-active --quiet orka-server
	sudo systemctl --no-pager --lines=20 status orka-server || true
fi
EOF

echo "Remote deployment completed for ${REMOTE_RELEASE_DIR}"
