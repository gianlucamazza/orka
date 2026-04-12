# orka-server

Main application binary for the Orka agent orchestration framework.

## Endpoints

| Method | Path                     | Description                                                   |
| ------ | ------------------------ | ------------------------------------------------------------- |
| `GET`  | `/health`                | Health summary                                                |
| `GET`  | `/health/live`           | Liveness probe (returns 200 when process is up)               |
| `GET`  | `/health/ready`          | Readiness probe (returns 200 when all subsystems are healthy) |
| `GET`  | `/docs`                  | Swagger UI (OpenAPI spec)                                     |
| `GET`  | `/api-doc/openapi.json`  | Raw OpenAPI JSON                                              |
| `GET`  | `/api/v1/version`        | Version metadata                                              |
| `GET`  | `/api/v1/info`           | Lightweight server capability summary                         |

Port `8080` serves the management and health API. Port `8081` serves the custom
adapter HTTP/WebSocket surface, including `POST /api/v1/message`.

## Starting

```bash
# With Docker Compose (recommended)
docker compose up -d

# Locally (requires Redis + Qdrant running)
just run
```

## Configuration

The server reads `orka.toml` (path overridden by `ORKA_CONFIG` env var) merged with
`ORKA__*` environment variables. See the repository root `orka.toml` for the
canonical sample configuration kept aligned with the current parser.

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
priority queue (Redis) ← sorted set via orka-infra
    │  pops
    ▼
orka-worker           ← WorkspaceHandler (LLM agentic loop)
    │  publishes reply to bus
    ▼
ChannelAdapter        ← delivers OutboundMessage back to user
```

See `docs/reference/architecture.md` for a detailed end-to-end description.
