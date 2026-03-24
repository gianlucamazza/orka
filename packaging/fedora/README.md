# Fedora Packaging Scaffold

This directory contains an RPM spec scaffold intended for Fedora and compatible RPM-based distributions.

## Goals

- package `orka` CLI and `orka-server`
- install configuration under `/etc/orka`
- install systemd unit, `sysusers.d`, and `tmpfiles.d` assets
- reuse upstream deployment files with minimal distro-specific patching

## Intended Validation

```bash
just package-lint-fedora
```

This validation runs in a dedicated Fedora container and uses Fedora-packaged
`cargo`, `rpmbuild`, and `rpmlint`, which matches current Fedora tooling more
closely than host-local AUR packages on Arch.

Packagers will likely still need to adapt `Source0`, changelog handling, and distro-specific dependency names.
