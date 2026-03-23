# Orka Documentation

Welcome to the Orka documentation. This directory contains detailed guides on the architecture, setup, and extensibility of the Orka platform.

## Getting Started

If you are new to Orka, you may want to start with the main repository [README](../README.md) for a high-level overview and quick start instructions.

## Guides

The documentation is organized into the following areas:

### Architecture & Operations
*   **[Architecture Diagram & Overview](architecture.md)**: End-to-end message flow and subsystem overview.
*   **[Deployment Guide](deployment.md)**: Instructions for running Orka under Docker, bare-metal with systemd, and observability setups.
*   **[Configuration Reference](configuration.md)**: Extensive documentation of all options within `orka.toml` and environment variables.
*   **[CLI Reference](cli-reference.md)**: Command-line tool reference for the `orka` binary.

### Agent & LLM Features
*   **[Prompt Architecture (Agents)](agents.md)**: Guide to the template-based prompt pipeline, overriding built-ins, and `SOUL.md`.
*   **[Experience System](experience-system.md)**: The continuous self-learning loop (reflection and distillation pipelines).
*   **[Model Context Protocol (MCP)](mcp-guide.md)**: Information about using Orka as either an MCP Client or an MCP Server.

### Development & Extensibility
*   **[Skill Development Guide](skill-development.md)**: Writing built-in Rust skills, WASM plugins, and markdown-based Soft skills (`SKILL.md`).
*   **[WASM Plugin Tutorial](tutorials/build-a-wasm-plugin.md)**: Step-by-step guide to writing WebAssembly modules for Orka.
*   **[Evaluation Framework](eval-guide.md)**: How to write and run `.eval.toml` integration tests for skills using `orka-eval`.
*   **[Adapters Guide](adapters-guide.md)**: Setup instructions for all supported inbound messaging channels (Telegram, Discord, Slack, WhatsApp, HTTP).

For contributing guidelines and security reporting, please check [CONTRIBUTING.md](../CONTRIBUTING.md) and [SECURITY.md](../SECURITY.md) in the project root.
