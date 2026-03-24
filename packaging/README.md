# Packaging Support Matrix

This directory contains native packaging scaffolding built on top of Orka's shared deployment assets in `deploy/`.

## Support Model

Orka follows a portable-first distribution strategy:

- **Tier 1**: portable upstream artifacts
  - GitHub release tarball
  - OCI image
  - generic `scripts/install.sh` systemd installation
- **Tier 2**: native distro packaging
  - Arch Linux via `PKGBUILD`
  - Debian packaging scaffold in `packaging/debian/`
  - Fedora/RHEL packaging scaffold in `packaging/fedora/`

Native packaging must respect the repository's Rust toolchain floor:

- `edition = 2024`
- `rust-version = 1.91`

Distributions that ship an older Rust toolchain need a newer backport, side tag,
or alternative toolchain source before native packaging is considered supported.

## Matrix

| Distribution family | Portable install | Native package status | Notes |
| ------------------- | ---------------- | --------------------- | ----- |
| Arch Linux          | Yes              | Implemented           | `PKGBUILD` in repository root; host toolchain already satisfies the Rust floor |
| Debian / Ubuntu     | Yes              | Scaffolded            | Debhelper layout under `packaging/debian/`; requires distro Rust >= 1.91 or a maintained newer toolchain |
| Fedora / RHEL       | Yes              | Scaffolded            | RPM spec under `packaging/fedora/`; requires distro Rust >= 1.91 or distro-approved newer toolchain |

## Validation Workflow

- Run `just msrv` to validate the official minimum supported Rust toolchain.
- Run `just package-lint-debian` to build and lint the Debian package in a dedicated Debian container.
- Run `just package-lint-fedora` to build and lint the RPM package in a dedicated Fedora container.
- Run `just package-lint` to execute both packaging validations.

The Debian container uses `rustup` internally because current Debian distro
toolchains lag behind Orka's upstream Rust floor. Fedora 42 already ships a new
enough Rust and `rpmlint`, so the RPM validation uses distro packages directly.

## Shared Packaging Rules

- Keep configuration in `/etc/orka`.
- Keep mutable state in `/var/lib/orka`.
- Reuse `deploy/orka-server.service`, `deploy/orka-server.sysusers`, and `deploy/orka-server.tmpfiles`.
- Patch the systemd unit's `@BINDIR@` placeholder at package build time.
- Keep upstream service units free of distro-specific service dependencies.

## Next Steps

1. Run packaging validation in CI on every pull request.
2. Decide whether native packages should be published as release artifacts.
3. Revisit Debian native package support once Debian-family distro toolchains catch up or a distro-compatible Rust source is chosen.
