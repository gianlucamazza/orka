#!/usr/bin/env bash
set -euo pipefail

IMAGE="${DEBIAN_LINT_IMAGE:-debian:trixie}"
PREFIX="\033[1;36m[orka-deb]\033[0m"
info() { echo -e "${PREFIX} $*"; }
error() { echo -e "${PREFIX} \033[1;31m$*\033[0m"; }

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORKDIR="$(mktemp -d /tmp/orka-debian-lint.XXXXXX)"
SOURCE_DIR="${WORKDIR}/src"

cleanup() {
	if [[ "${KEEP_PACKAGING_WORKDIR:-0}" != "1" ]]; then
		rm -rf "$WORKDIR"
	else
		info "Preserved workdir: $WORKDIR"
	fi
}
trap cleanup EXIT

REQUIRED_RUST="$(grep -oP 'rust-version\s*=\s*"\K[^"]+' "$REPO_ROOT/Cargo.toml" | head -n1 || true)"
if [[ -z "$REQUIRED_RUST" ]]; then
	error "Unable to determine rust-version from Cargo.toml"
	exit 1
fi

mkdir -p "$SOURCE_DIR"
info "Creating source snapshot in $WORKDIR"
tar \
	--exclude=.git \
	--exclude=target \
	--exclude=.pytest_cache \
	--exclude=.fastembed_cache \
	-cf - \
	-C "$REPO_ROOT" . | tar -xf - -C "$SOURCE_DIR"

info "Running Debian packaging build and lint in $IMAGE"
docker run --rm \
	-v "$WORKDIR:/work" \
	-w /work/src \
	-e DEBIAN_FRONTEND=noninteractive \
	-e REQUIRED_RUST="$REQUIRED_RUST" \
	"$IMAGE" \
	bash -lc '
set -euo pipefail
apt-get update
apt-get install -y ca-certificates curl build-essential pkg-config libssl-dev debhelper fakeroot dh-make lintian
curl --proto "=https" --tlsv1.2 -sSf https://sh.rustup.rs | sh -s -- -y --profile minimal --default-toolchain "$REQUIRED_RUST"
. "$HOME/.cargo/env"
cp -r packaging/debian debian
RUSTC_WRAPPER= dpkg-buildpackage -us -uc -b
lintian --fail-on error --display-info /work/*.changes
'

info "Debian packaging lint completed successfully"
