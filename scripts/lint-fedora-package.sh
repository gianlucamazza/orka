#!/usr/bin/env bash
set -euo pipefail

# Fedora 42 distro rust is < MSRV 1.96. 43+ ships rustc >= 1.96.
IMAGE="${FEDORA_LINT_IMAGE:-fedora:43}"
PREFIX="\033[1;36m[orka-rpm]\033[0m"
info() { echo -e "${PREFIX} $*"; }
error() { echo -e "${PREFIX} \033[1;31m$*\033[0m"; }

# Host cleanup is on in GitHub Actions (CI=true) and when
# FEDORA_LINT_FREE_DISK=1. Set FEDORA_LINT_FREE_DISK=0 to skip.
# FEDORA_LINT_PRUNE_PATHS is a space-separated list of unused host paths.
FEDORA_LINT_FREE_DISK="${FEDORA_LINT_FREE_DISK:-}"
FEDORA_LINT_PRUNE_PATHS="${FEDORA_LINT_PRUNE_PATHS:-/usr/share/dotnet /usr/local/lib/android /opt/ghc /opt/hostedtoolcache /usr/local/share/powershell /usr/share/swift /usr/local/share/chromium /opt/az /usr/share/miniconda}"

should_free_disk() {
	case "${FEDORA_LINT_FREE_DISK}" in
	1 | true | TRUE | yes | YES) return 0 ;;
	0 | false | FALSE | no | NO) return 1 ;;
	esac
	[[ "${CI:-}" == "true" ]]
}

free_host_disk() {
	should_free_disk || return 0
	info "Freeing unused host toolchains/images before Fedora container build"
	df -h / /tmp 2>/dev/null || df -h

	local -a rm_cmd=(rm -rf)
	if [[ "$(id -u)" -ne 0 ]]; then
		if command -v sudo >/dev/null 2>&1 && sudo -n true 2>/dev/null; then
			rm_cmd=(sudo rm -rf)
		else
			info "Skipping privileged host path cleanup (no passwordless sudo)"
			rm_cmd=()
		fi
	fi

	if ((${#rm_cmd[@]})); then
		local path
		for path in ${FEDORA_LINT_PRUNE_PATHS}; do
			if [[ -e "${path}" ]]; then
				info "Removing ${path}"
				"${rm_cmd[@]}" "${path}" || true
			fi
		done
	fi

	if command -v docker >/dev/null 2>&1; then
		docker image prune -af >/dev/null 2>&1 || true
	fi

	df -h / /tmp 2>/dev/null || df -h
}

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
WORKDIR="$(mktemp -d /tmp/orka-fedora-lint.XXXXXX)"
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

free_host_disk

mkdir -p "$RPMROOT"
info "Creating source snapshot in $WORKDIR"
# One compressed tarball only — a second uncompressed tree previously
# duplicated the workspace on the same disk the Fedora build uses.
tar \
	--exclude=.git \
	--exclude=target \
	--exclude=.pytest_cache \
	--exclude=.fastembed_cache \
	-czf "$WORKDIR/orka-${VERSION}.tar.gz" \
	--transform "s,^,orka-${VERSION}/," \
	-C "$REPO_ROOT" .
cp "$REPO_ROOT/packaging/fedora/orka.spec" "$WORKDIR/orka.spec"

info "Running Fedora packaging build and lint in $IMAGE"
docker run --rm \
	-v "$WORKDIR:/work" \
	-w /work \
	"$IMAGE" \
	bash -lc "
set -euo pipefail
dnf -y install rust cargo gcc openssl-devel fontconfig-devel pkgconf-pkg-config rpm-build rpmlint systemd systemd-devel 'pkgconfig(systemd)' systemd-rpm-macros tar gzip
dnf -y clean all
rm -rf /var/cache/dnf /var/cache/rpm
mkdir -p /work/tmp /work/rpmbuild/{BUILD,BUILDROOT,RPMS,SOURCES,SPECS,SRPMS}
cp /work/orka-${VERSION}.tar.gz /work/rpmbuild/SOURCES/
cp /work/orka.spec /work/rpmbuild/SPECS/orka.spec
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=gcc \
CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUSTFLAGS= \
CARGO_PROFILE_RELEASE_LTO=false \
CARGO_PROFILE_RELEASE_CODEGEN_UNITS=16 \
CARGO_BUILD_JOBS=2 \
CARGO_INCREMENTAL=0 \
TMPDIR=/work/tmp \
RUSTFLAGS= \
RUSTC_WRAPPER= \
rpmbuild -ba --define '_topdir /work/rpmbuild' --define '_tmppath /work/tmp' --define \"pkg_version ${VERSION}\" /work/rpmbuild/SPECS/orka.spec
rpmlint /work/rpmbuild/SRPMS/*.src.rpm /work/rpmbuild/RPMS/*/*.rpm
"

info "Fedora packaging lint completed successfully"
