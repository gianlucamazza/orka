# orka-server

Main application binary for the Orka agent orchestration framework.

## Endpoints

| Method | Path                     | Description                                                   |
| ------ | ------------------------ | ------------------------------------------------------------- |
| `POST` | `/message`               | Submit an inbound message for agent processing                |
| `GET`  | `/health/live`           | Liveness probe (returns 200 when process is up)               |
| `GET`  | `/health/ready`          | Readiness probe (returns 200 when all subsystems are healthy) |
| `GET`  | `/docs`                  | Swagger UI (OpenAPI spec)                                     |
| `GET`  | `/api-docs/openapi.json` | Raw OpenAPI JSON                                              |

Port `8080` serves the REST API. Port `8081` serves the custom adapter webhook.

## Starting

```bash
# With Docker Compose (recommended)
docker compose up -d

# Locally (requires Redis + Qdrant running)
just run
```

## Configuration

The server reads `orka.toml` (path overridden by `ORKA_CONFIG` env var) merged with
`ORKA__*` environment variables. See the root `orka.toml` for a fully-commented
reference configuration.

## Architecture

```
HTTP request
    │
    ▼
orka-server (axum)
    │  publishes Envelope to bus
    ▼
orka-gateway          ← dedup, rate-limit, session resolution
    │  pushes to queue
    ▼
orka-queue (Redis)    ← priority sorted set
    │  pops
    ▼
orka-worker           ← WorkspaceHandler (LLM agentic loop)
    │  publishes reply to bus
    ▼
ChannelAdapter        ← delivers OutboundMessage back to user
```

See `docs/architecture.md` for a detailed end-to-end description.
