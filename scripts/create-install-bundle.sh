#!/usr/bin/env bash
set -euo pipefail

usage() {
	cat <<'EOF'
Usage: create-install-bundle.sh [OPTIONS]

Create a portable install bundle that preserves the current script-driven
systemd install flow.

OPTIONS:
  --profile PROFILE          Cargo profile layout to embed (default: release)
  --source-bin-dir DIR       Directory containing prebuilt orka binaries
  --output-dir DIR           Bundle output directory
  --tarball PATH             Optional .tar.gz path to create after bundling
  --help, -h                 Show this help

Examples:
  ./scripts/create-install-bundle.sh
  ./scripts/create-install-bundle.sh \
    --source-bin-dir target/aarch64-unknown-linux-gnu/release \
    --output-dir dist/orka-install-bundle-arm64 \
    --tarball dist/orka-install-bundle-arm64.tar.gz
EOF
}

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PROFILE="release"
SOURCE_BIN_DIR=""
OUTPUT_DIR=""
TARBALL=""

while [[ $# -gt 0 ]]; do
	case "$1" in
	--profile)
		shift
		PROFILE="${1:?--profile requires a value}"
		;;
	--profile=*) PROFILE="${1#--profile=}" ;;
	--source-bin-dir)
		shift
		SOURCE_BIN_DIR="${1:?--source-bin-dir requires a value}"
		;;
	--source-bin-dir=*) SOURCE_BIN_DIR="${1#--source-bin-dir=}" ;;
	--output-dir)
		shift
		OUTPUT_DIR="${1:?--output-dir requires a value}"
		;;
	--output-dir=*) OUTPUT_DIR="${1#--output-dir=}" ;;
	--tarball)
		shift
		TARBALL="${1:?--tarball requires a value}"
		;;
	--tarball=*) TARBALL="${1#--tarball=}" ;;
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

if [[ -z "$SOURCE_BIN_DIR" ]]; then
	SOURCE_BIN_DIR="${REPO_ROOT}/target/${PROFILE}"
fi

if [[ -z "$OUTPUT_DIR" ]]; then
	OUTPUT_DIR="${REPO_ROOT}/dist/orka-install-bundle-${PROFILE}"
fi

SERVER_BIN="${SOURCE_BIN_DIR}/orka-server"
CLI_BIN="${SOURCE_BIN_DIR}/orka"

[[ -x "$SERVER_BIN" ]] || {
	echo "Missing executable: $SERVER_BIN" >&2
	exit 1
}
[[ -x "$CLI_BIN" ]] || {
	echo "Missing executable: $CLI_BIN" >&2
	exit 1
}

VERSION="$(sed -n 's/^version *= *"\(.*\)"/\1/p' "${REPO_ROOT}/Cargo.toml" | head -n1)"
GIT_REV="$(git -C "${REPO_ROOT}" describe --tags --always --dirty 2>/dev/null || git -C "${REPO_ROOT}" rev-parse --short HEAD)"

TMP_DIR="$(mktemp -d)"
BUNDLE_DIR="${TMP_DIR}/bundle"
trap 'rm -rf "${TMP_DIR}"' EXIT

mkdir -p "${BUNDLE_DIR}/target/${PROFILE}" "${BUNDLE_DIR}/scripts" "${BUNDLE_DIR}/dist"

cp -p "${SERVER_BIN}" "${BUNDLE_DIR}/target/${PROFILE}/orka-server"
cp -p "${CLI_BIN}" "${BUNDLE_DIR}/target/${PROFILE}/orka"
cp -p "${REPO_ROOT}/Cargo.toml" "${BUNDLE_DIR}/Cargo.toml"
cp -p "${REPO_ROOT}/Cargo.lock" "${BUNDLE_DIR}/Cargo.lock"
cp -p "${REPO_ROOT}/orka.toml" "${BUNDLE_DIR}/orka.toml"
cp -p "${REPO_ROOT}/scripts/install.sh" "${BUNDLE_DIR}/scripts/install.sh"
cp -a "${REPO_ROOT}/deploy" "${BUNDLE_DIR}/deploy"
cp -a "${REPO_ROOT}/workspaces" "${BUNDLE_DIR}/workspaces"
mkdir -p "${BUNDLE_DIR}/assets"
cp -a "${REPO_ROOT}/assets/desktop" "${BUNDLE_DIR}/assets/desktop"
cp -a "${REPO_ROOT}/assets/icons" "${BUNDLE_DIR}/assets/icons"

cat > "${BUNDLE_DIR}/dist/install-bundle-info" <<EOF
version=${VERSION:-unknown}
git_rev=${GIT_REV:-unknown}
profile=${PROFILE}
source_bin_dir=${SOURCE_BIN_DIR}
built_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)
EOF

OUTPUT_DIR_ABS="$(realpath -m "${OUTPUT_DIR}")"
mkdir -p "$(dirname "${OUTPUT_DIR_ABS}")"
rm -rf "${OUTPUT_DIR_ABS}"
mv "${BUNDLE_DIR}" "${OUTPUT_DIR_ABS}"

if [[ -n "${TARBALL}" ]]; then
	TARBALL_ABS="$(realpath -m "${TARBALL}")"
	mkdir -p "$(dirname "${TARBALL_ABS}")"
	tar -czf "${TARBALL_ABS}" -C "$(dirname "${OUTPUT_DIR_ABS}")" "$(basename "${OUTPUT_DIR_ABS}")"
fi

echo "Bundle created at ${OUTPUT_DIR_ABS}"
if [[ -n "${TARBALL}" ]]; then
	echo "Tarball created at ${TARBALL_ABS}"
fi
