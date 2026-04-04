# Orka Documentation

Welcome to the Orka documentation. This directory contains detailed guides on the architecture, setup, and extensibility of the Orka platform.

## Getting Started

If you are new to Orka, you may want to start with the main repository [README](../README.md) for a high-level overview and quick start instructions.

## Start Here

- **Run or deploy Orka**: start with [Deployment Guide](reference/deployment.md), then [Configuration Reference](reference/configuration.md).
- **Operate an existing instance**: use [CLI Reference](reference/cli-reference.md), [Architecture Diagram & Overview](reference/architecture.md), and [Security](../SECURITY.md).
- **Build skills or plugins**: use [Skill Development Guide](guides/skill-development.md), [WASM Plugin Tutorial](guides/tutorials/build-a-wasm-plugin.md), and [orka-plugin-sdk](../sdk/orka-plugin-sdk/README.md).
- **Work on the codebase**: start with [Contributing Guide](../CONTRIBUTING.md), then [Architecture Diagram & Overview](reference/architecture.md), [examples/](../examples/README.md), and selected crate READMEs below.

## Layout

This index is the canonical navigation hub. Public/stable docs, preview docs,
internal snapshots, and repo companion docs are listed separately on purpose.

### Reference
*   **[Architecture Diagram & Overview](reference/architecture.md)**: End-to-end message flow and subsystem overview.
*   **[Architecture Principles](reference/architecture-principles.md)**: Normative layering, modularity, and dependency rules for the workspace.
*   **[Deployment Guide](reference/deployment.md)**: Instructions for running Orka under Docker, bare-metal with systemd, and observability setups.
*   **[Configuration Reference](reference/configuration.md)**: Top-level `orka.toml` schema, key fields, and pointers to the canonical sample config.
*   **[CLI Reference](reference/cli-reference.md)**: Command-line tool reference for the currently exposed `orka` parser.
*   **[Model Context Protocol (MCP)](reference/mcp-guide.md)**: Information about using Orka as either an MCP Client or an MCP Server.

### Guides
*   **[Prompt Architecture (Agents)](guides/agents.md)**: Guide to the template-based prompt pipeline, overriding built-ins, and `SOUL.md`.
*   **[Mobile Client Guide](guides/mobile-client.md)**: Product-facing mobile API, auth model, and streaming contract.
*   **[Experience System](guides/experience-system.md)**: The continuous self-learning loop (reflection and distillation pipelines).
*   **[Skill Development Guide](guides/skill-development.md)**: Writing built-in Rust skills, WASM plugins, and markdown-based Soft skills (`SKILL.md`).
*   **[WASM Plugin Tutorial](guides/tutorials/build-a-wasm-plugin.md)**: Step-by-step guide to writing WebAssembly modules for Orka.
*   **[Evaluation Framework](guides/eval-guide.md)**: How to write and run `.eval.toml` integration tests for skills using `orka-eval`.

### Companion Docs
*   **[Examples](../examples/README.md)**: Runnable examples covering bots, custom skills, WASM plugins, and multi-agent flows.
*   **[Demo Pipeline](../demo/README.md)**: How public GIF/MP4/WebM demo assets are recorded, rendered, and verified.
*   **[Packaging](../packaging/README.md)**: Native packaging support matrix and distro-specific packaging notes.
*   **[Plugin SDK](../sdk/orka-plugin-sdk/README.md)**: Low-level SDK reference for building WASM plugins.
*   **[Tooling Layout](../tools/README.md)**: Helper utilities that live outside the Rust workspace.
*   **[Test Layout](../tests/README.md)**: Workspace-level testing conventions for end-to-end and cross-crate tests.

### Selected Crate Docs
*   **[orka-server](../crates/orka-server/README.md)**: Server endpoints and runtime entrypoint notes.
*   **[orka-worker](../crates/orka-worker/README.md)**: Worker pool and retry behavior.
*   **[orka-gateway](../crates/orka-gateway/README.md)**: Inbound gateway responsibilities and flow.
*   **[orka-core](../crates/orka-core/README.md)**: Shared types and core contracts.
*   **[orka-llm](../crates/orka-llm/README.md)**: Provider routing and client behavior.
*   **[orka-eval](../crates/orka-eval/README.md)**: Eval runner internals and usage.
*   **[orka-adapter-telegram](../crates/orka-adapter-telegram/README.md)**: Telegram-specific adapter behavior and metadata.

Crate-local READMEs are intentionally selective; not every workspace crate has one.

### Preview
*   **[Research Workflow (Internal Preview)](guides/research-workflow.md)**: Experimental research subsystem notes; not part of the current public CLI surface.

### Internal Snapshots
*   **[Analysis Report](internal/analysis-report.md)**: Repository analysis and current-state notes.
*   **[Architecture Analysis (2026-03-25)](internal/architecture-analysis-2026-03-25.md)**: Point-in-time architectural review and layering analysis.
*   **[Root Organization Decisions](internal/root-organization-decisions.md)**: Decision record for special root files, workspace model, packaging, and test layout.
*   **[Security Report](internal/security-report.md)**: Internal security findings and remediation priorities.
*   **[Test Coverage Report](internal/test-coverage-report.md)**: Current testing coverage inventory and gaps.

### Contributing
*   **[Contributing Guide](../CONTRIBUTING.md)**: Development setup, coding standards, Rust best practices, and PR guidelines.
*   **[Security](../SECURITY.md)**: Security reporting and policies.
