# Orka Documentation

Welcome to the Orka documentation. This directory contains detailed guides on the architecture, setup, and extensibility of the Orka platform.

## Getting Started

If you are new to Orka, you may want to start with the main repository [README](../README.md) for a high-level overview and quick start instructions.

## Layout

The documentation is organized into the following areas:

### Reference
*   **[Architecture Diagram & Overview](reference/architecture.md)**: End-to-end message flow and subsystem overview.
*   **[Deployment Guide](reference/deployment.md)**: Instructions for running Orka under Docker, bare-metal with systemd, and observability setups.
*   **[Configuration Reference](reference/configuration.md)**: All `orka.toml` options, environment variables, and adapter setups (Telegram, Discord, Slack, WhatsApp, HTTP).
*   **[CLI Reference](reference/cli-reference.md)**: Command-line tool reference for the `orka` binary.
*   **[Model Context Protocol (MCP)](reference/mcp-guide.md)**: Information about using Orka as either an MCP Client or an MCP Server.

### Guides
*   **[Prompt Architecture (Agents)](guides/agents.md)**: Guide to the template-based prompt pipeline, overriding built-ins, and `SOUL.md`.
*   **[Experience System](guides/experience-system.md)**: The continuous self-learning loop (reflection and distillation pipelines).
*   **[Skill Development Guide](guides/skill-development.md)**: Writing built-in Rust skills, WASM plugins, and markdown-based Soft skills (`SKILL.md`).
*   **[WASM Plugin Tutorial](guides/tutorials/build-a-wasm-plugin.md)**: Step-by-step guide to writing WebAssembly modules for Orka.
*   **[Evaluation Framework](guides/eval-guide.md)**: How to write and run `.eval.toml` integration tests for skills using `orka-eval`.

### Internal
*   **[Analysis Report](internal/analysis-report.md)**: Repository analysis and current-state notes.
*   **[Root Organization Decisions](internal/root-organization-decisions.md)**: Decision record for special root files, workspace model, packaging, and test layout.

### Contributing
*   **[Contributing Guide](../CONTRIBUTING.md)**: Development setup, coding standards, Rust best practices, and PR guidelines.
*   **[Security](../SECURITY.md)**: Security reporting and policies.

See also the [examples/](../examples/) directory for working code samples.
