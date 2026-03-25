#!/usr/bin/env bash
set -euo pipefail

IMAGE="${FEDORA_LINT_IMAGE:-fedora:42}"
PREFIX="\033[1;36m[orka-rpm]\033[0m"
info() { echo -e "${PREFIX} $*"; }
error() { echo -e "${PREFIX} \033[1;31m$*\033[0m"; }

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORKDIR="$(mktemp -d /tmp/orka-fedora-lint.XXXXXX)"
SOURCE_DIR="${WORKDIR}/src"
RPMROOT="${WORKDIR}/rpmbuild"

cleanup() {
	if [[ "${KEEP_PACKAGING_WORKDIR:-0}" != "1" ]]; then
		chmod -R u+rwX "$WORKDIR" 2>/dev/null || true
		rm -rf "$WORKDIR" 2>/dev/null || true
	else
		info "Preserved workdir: $WORKDIR"
	fi
}
trap cleanup EXIT

VERSION="$(
	awk '
		$0 == "[workspace.package]" { in_section = 1; next }
		/^\[/ { in_section = 0 }
		in_section && $1 == "version" {
			gsub(/"/, "", $3)
			print $3
			exit
		}
	' "$REPO_ROOT/Cargo.toml"
)"
if [[ -z "$VERSION" ]]; then
	error "Unable to determine workspace version from Cargo.toml"
	exit 1
fi

mkdir -p "$SOURCE_DIR" "$RPMROOT"
info "Creating source snapshot in $WORKDIR"
tar \
	--exclude=.git \
	--exclude=target \
	--exclude=.pytest_cache \
	--exclude=.fastembed_cache \
	-cf - \
	-C "$REPO_ROOT" . | tar -xf - -C "$SOURCE_DIR"
tar \
	--exclude=.git \
	--exclude=target \
	--exclude=.pytest_cache \
	--exclude=.fastembed_cache \
	-czf "$WORKDIR/orka-${VERSION}.tar.gz" \
	--transform "s,^,orka-${VERSION}/," \
	-C "$REPO_ROOT" .

info "Running Fedora packaging build and lint in $IMAGE"
docker run --rm \
	-v "$WORKDIR:/work" \
	-w /work \
	"$IMAGE" \
	bash -lc "
set -euo pipefail
dnf -y install cargo gcc openssl-devel pkgconf-pkg-config rpm-build rpmlint systemd-devel systemd-rpm-macros tar gzip
mkdir -p /work/rpmbuild/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}
cp /work/orka-${VERSION}.tar.gz /work/rpmbuild/SOURCES/
cp /work/src/packaging/fedora/orka.spec /work/rpmbuild/SPECS/orka.spec
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=gcc \
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS= \
RUSTFLAGS= \
RUSTC_WRAPPER= \
rpmbuild -ba --define '_topdir /work/rpmbuild' --define "pkg_version ${VERSION}" /work/rpmbuild/SPECS/orka.spec
rpmlint /work/rpmbuild/SRPMS/*.src.rpm /work/rpmbuild/RPMS/*/*.rpm
"

info "Fedora packaging lint completed successfully"
