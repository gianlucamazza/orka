# Changelog

All notable changes to Orka will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/), and this project adheres to [Semantic Versioning](https://semver.org/).

## [0.1.0] - 2026-03-16

### Added

- Multi-channel agent orchestration (Telegram, Discord, Slack, WhatsApp, custom HTTP/WebSocket)
- Priority queue with Redis Sorted Sets (Urgent, Normal, Background)
- LLM integration with Anthropic Claude and OpenAI (streaming support)
- Skill system with registry, schema validation, and WASM plugin support
- MCP (Model Context Protocol) server with JSON-RPC 2.0 over stdio
- A2A (Agent-to-Agent) protocol for inter-agent communication
- Agent router with prefix-based routing and delegation
- Workspace-based agent configuration with hot-reload (SOUL.md, IDENTITY.md)
- Session management with Redis-backed storage
- Memory store (key-value per session)
- Secret management with AES-256-GCM encryption at rest
- Circuit breaker pattern for external service resilience
- Guardrails for input/output validation and content filtering
- Sandboxed code execution (process isolation and WASM)
- Knowledge base with RAG (Qdrant vector store, document ingestion)
- Cron-based task scheduler
- OS integration skills (filesystem, process, system info)
- HTTP request skill with SSRF protection
- Rate limiting and message deduplication at the gateway
- JWT and API key authentication
- OpenTelemetry tracing and Prometheus metrics
- Swagger UI for API documentation
- CLI tool for workspace management
- Docker Compose deployment
- Plugin SDK for WASM-based extensions
