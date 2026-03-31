#!/usr/bin/env bash
set -euo pipefail

usage() {
	cat <<'EOF'
Usage: publish-oci.sh

Publish the Orka OCI image to the homelab registry using the current Drone
commit SHA and the preconfigured multi-arch buildx builder.

Required environment:
  REGISTRY_USERNAME
  REGISTRY_PASSWORD
  DRONE_COMMIT_SHA

Optional environment:
  OCI_REGISTRY_HOST   Registry host (default: registry.home.gianlucamazza.it)
  OCI_IMAGE_REPO      Full image repo (default: registry.home.gianlucamazza.it/gmazza/orka)
  OCI_IMAGE_TAG       Commit/image version tag (default: DRONE_COMMIT_SHA)
  OCI_IMAGE_LATEST    Latest tag alias (default: latest)
  OCI_BUILDER         buildx builder name (default: orka-builder)
EOF
}

if [[ "${1:-}" == "--help" || "${1:-}" == "-h" ]]; then
	usage
	exit 0
fi

: "${REGISTRY_USERNAME:?REGISTRY_USERNAME is required}"
: "${REGISTRY_PASSWORD:?REGISTRY_PASSWORD is required}"
: "${DRONE_COMMIT_SHA:?DRONE_COMMIT_SHA is required}"

OCI_REGISTRY_HOST="${OCI_REGISTRY_HOST:-registry.home.gianlucamazza.it}"
OCI_IMAGE_REPO="${OCI_IMAGE_REPO:-registry.home.gianlucamazza.it/gmazza/orka}"
OCI_IMAGE_TAG="${OCI_IMAGE_TAG:-${DRONE_COMMIT_SHA}}"
OCI_IMAGE_LATEST="${OCI_IMAGE_LATEST:-latest}"
OCI_BUILDER="${OCI_BUILDER:-orka-builder}"
BUILD_DATE="$(date -u +%Y-%m-%dT%H:%M:%SZ)"

printf '%s' "${REGISTRY_PASSWORD}" | docker login "${OCI_REGISTRY_HOST}" --username "${REGISTRY_USERNAME}" --password-stdin
docker buildx inspect "${OCI_BUILDER}" --bootstrap
docker buildx build \
	--builder "${OCI_BUILDER}" \
	--platform linux/amd64,linux/arm64 \
	--build-arg "BUILD_DATE=${BUILD_DATE}" \
	--build-arg "VCS_REF=${DRONE_COMMIT_SHA}" \
	--build-arg "VERSION=${OCI_IMAGE_TAG}" \
	-t "${OCI_IMAGE_REPO}:${OCI_IMAGE_TAG}" \
	-t "${OCI_IMAGE_REPO}:${OCI_IMAGE_LATEST}" \
	--push \
	.
