# orka-config

Composition root for Orka configuration.

This crate owns:

- `OrkaConfig`
- config loading from TOML + environment
- validation orchestration
- config migration and schema inspection entrypoints

Domain-specific config sections stay in their owning crates and are re-exported
here so binaries and composition crates can depend on a single canonical
configuration surface.
