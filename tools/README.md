# Tooling Layout

This directory contains standalone helper utilities that are not part of the Rust workspace itself.

- Use `tools/` for utilities with their own runtime, package manager, or toolchain.
- Use `scripts/` for shell-based automation, setup, packaging, and installation flows.
- Keep each tool self-contained in its own subdirectory with its local manifest files.

Current contents:

- `claude-channel/`: MCP bridge for Claude Code and Orka integration, implemented with TypeScript/Bun.
