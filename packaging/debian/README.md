# Debian Packaging Scaffold

This directory contains a debhelper-based packaging scaffold for Debian and Ubuntu derivatives.

The scaffold assumes a build environment that already satisfies Orka's minimum
Rust toolchain (`rust-version = 1.91`, `edition = 2024`). Debian 12
(`bookworm`) and older Ubuntu releases do not meet that requirement with their
default distro `rustc`/`cargo` packages.

## Goals

- package `orka` CLI and `orka-server`
- install configuration in `/etc/orka`
- install systemd unit, `sysusers.d`, and `tmpfiles.d` assets
- preserve local config changes on upgrade

## Intended Build Flow

Typical usage from a source package layout:

```bash
cp -r packaging/debian ./debian
dpkg-buildpackage -us -uc
```

For repository-local validation on any host, prefer the containerized workflow:

```bash
just package-lint-debian
```

## Notes

- `dh_installsystemd` should manage service lifecycle integration.
- `orka.toml` should be shipped as a conffile under `/etc/orka/orka.toml`.
- A distro-maintained package may need to adjust runtime dependencies and package names for Redis or Valkey.
- The containerized validation flow installs the required Rust version via `rustup` because Debian-family distro toolchains are currently behind Orka's upstream minimum.
- For an officially supported distro-native package, use a maintained backport, a distro-approved newer Rust source, or wait for Debian-family releases to meet the Rust floor directly.
