# Mobile Client Guide

The mobile app should use Orka's dedicated product-facing API, not the
management routes and not the legacy `custom` adapter surface.

## Auth Model

- Mobile routes are exposed only when JWT auth is configured.
- The authenticated JWT subject (`sub`) becomes the owning `user_id`.
- The mobile client must send `Authorization: Bearer <token>`.
- QR pairing and refresh are available only when Orka is configured with
  `auth.jwt.secret`. A server configured only with `public_key_path` can
  validate mobile JWTs but cannot issue new device sessions.
- The recommended first association flow is:
  1. Authenticated CLI calls `POST /mobile/v1/pairings`
  2. CLI renders the returned `mobileorka://pair?...` URI as a QR code
  3. Mobile app calls `POST /mobile/v1/pairings/complete`
  4. App later rotates credentials through `POST /mobile/v1/auth/refresh`

## Endpoints

Base path: `/mobile/v1`

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/me` | Return the authenticated user profile (`user_id`, `scopes`) |
| `POST` | `/pairings` | Create a one-time pairing session for an authenticated CLI/operator caller |
| `GET` | `/pairings/{id}` | Poll pairing status from the CLI |
| `POST` | `/pairings/complete` | Complete pairing from the mobile app and receive access + refresh tokens |
| `POST` | `/auth/refresh` | Rotate an existing refresh token |
| `GET` | `/conversations` | List recent conversations for the current user |
| `POST` | `/conversations` | Create an empty conversation |
| `GET` | `/conversations/{id}` | Fetch conversation metadata |
| `POST` | `/uploads` | Upload a pending artifact owned by the current user |
| `GET` | `/artifacts/{id}` | Fetch artifact metadata |
| `DELETE` | `/artifacts/{id}` | Delete an unattached uploaded artifact |
| `GET` | `/artifacts/{id}/content` | Fetch artifact bytes with bearer auth |
| `GET` | `/conversations/{id}/messages` | Fetch transcript messages |
| `POST` | `/conversations/{id}/messages` | Append a user message and enqueue Orka processing |
| `POST` | `/conversations/{id}/read` | Advance the read watermark for the conversation |
| `GET` | `/conversations/{id}/stream` | Stream assistant progress and completion events as SSE |
| `PATCH` | `/conversations/{id}` | Update conversation metadata (archive, rename, pin, tags) |
| `DELETE` | `/conversations/{id}` | Permanently delete a conversation and its transcript |
| `POST` | `/conversations/{id}/cancel` | Cancel an in-progress generation |
| `DELETE` | `/conversations/{id}/messages/{message_id}` | Delete a single message from the transcript |
| `POST` | `/conversations/{id}/retry` | Retry the last failed generation |

Pagination:

- `GET /conversations` accepts `limit` and `offset`.
  Default `limit = 20`, maximum `limit = 100`.
- `GET /conversations/{id}/messages` uses cursor-based pagination.
  Parameters: `limit` (default 20, capped at 100), `after` (exclusive lower bound cursor),
  `before` (exclusive upper bound cursor). `after` and `before` are mutually exclusive.
  Response headers: `x-next-cursor` (cursor past the last returned message),
  `x-prev-cursor` (cursor before the first returned message).
  To load older history: request with `before=<x-prev-cursor>`.
  To load newer messages: request with `after=<x-next-cursor>`.

Pairing semantics:

- Pairing URIs use the `mobileorka://pair` scheme and carry:
  - `server`
  - `pairing_id`
  - `pairing_secret`
- Pairing sessions are one-time and short-lived.
- Expired or already-consumed pairing sessions return `410 Gone`.
- Refresh tokens are opaque, device-scoped, server-stored, and rotated on each
  successful refresh.

## Streaming Contract

The stream endpoint emits Server-Sent Events with these event names:

- `typing_started`
  Data: `{ "conversation_id" }`
  Emitted when the model begins generating. Useful for showing a typing indicator.
- `message_delta`
  Data: `{ "conversation_id", "reply_to"?, "delta" }`
  Carries incremental assistant text while the model is responding.
- `thinking_delta`
  Data: `{ "conversation_id", "delta" }`
  Carries incremental reasoning/thinking text (only when extended thinking is enabled).
- `tool_exec_start`
  Data: `{ "conversation_id", "id", "name", "input_summary"?, "category"? }`
  Emitted when Orka begins executing a skill or tool.
- `tool_exec_end`
  Data: `{ "conversation_id", "id", "success", "duration_ms", "error"?, "result_summary"? }`
  Emitted when Orka finishes executing a skill or tool.
- `agent_switch`
  Data: `{ "conversation_id", "display_name" }`
  Emitted when the multi-agent graph transitions to a different agent.
- `artifact_ready`
  Data: `{ "conversation_id", "artifact": ConversationArtifact }`
  Emitted when an assistant output artifact has been durably persisted and can
  already be fetched through the content endpoint.
- `message_completed`
  Data: `{ "message": ConversationMessage }`
  Emitted only after the assistant message has been persisted to the transcript.
- `message_failed`
  Data: `{ "conversation_id", "error" }`
  Indicates a terminal generation failure.
- `stream_done`
  Data: `{ "conversation_id" }`
  Only indicates that the transport stream finished; it does not imply success.

Client rules:

- Treat transcript reload as the source of truth after reconnect, app resume,
  or any missing terminal event.
- Treat artifact metadata as transcript data and binary content as a separate
  authenticated fetch through `/artifacts/{id}/content`.
- If `message_failed` is received, refresh both conversation metadata and the
  transcript.
- If `stream_done` arrives without `message_completed`, reload the transcript
  and conversation list to observe any durable status change such as
  `interrupted`.

## Conversation Model

- `Conversation` is the product-facing chat thread exposed to clients.
- `Session` remains the runtime orchestration concept used internally by Orka.
- In the current implementation, each mobile conversation has a 1:1 backing
  runtime session and shares the same underlying UUID value, but the API
  models are intentionally distinct.
- `Conversation.status` is durable server state:
  - `active` for normal operation
  - `interrupted` when Orka paused for human or external input
  - `failed` when the last generation terminated with an error
- `ConversationMessage.artifacts` is the durable attachment list associated
  with that message. Artifact bytes are never inlined into transcript payloads.

## Examples

`GET /mobile/v1/conversations?limit=20&offset=0`

```json
[
  {
    "id": "0a8d3f4b-0c3c-4837-98b4-bc2a71ee2cd1",
    "session_id": "0a8d3f4b-0c3c-4837-98b4-bc2a71ee2cd1",
    "user_id": "user-123",
    "title": "Fix the onboarding flow",
    "last_message_preview": "I paused at the approval step.",
    "status": "interrupted",
    "created_at": "2026-04-01T09:30:00Z",
    "updated_at": "2026-04-01T09:34:12Z"
  }
]
```

`POST /mobile/v1/conversations/{id}/messages`

```json
{
  "text": "Continue from the last checkpoint.",
  "artifact_ids": ["0195f640-3a61-75df-933f-c366d6c1d2a5"]
}
```

```json
{
  "conversation_id": "0a8d3f4b-0c3c-4837-98b4-bc2a71ee2cd1",
  "session_id": "0a8d3f4b-0c3c-4837-98b4-bc2a71ee2cd1",
  "message_id": "9f78ad29-8aa1-4e3b-9633-c348a8cc3d74"
}
```

`message_failed`

```json
{
  "conversation_id": "0a8d3f4b-0c3c-4837-98b4-bc2a71ee2cd1",
  "error": "agent execution terminated with error"
}
```

`POST /mobile/v1/uploads`

```text
multipart/form-data
  file=<binary>
  caption?=<optional string>
```

```json
{
  "artifact": {
    "id": "0195f640-3a61-75df-933f-c366d6c1d2a5",
    "owner_user_id": "user-123",
    "conversation_id": null,
    "message_id": null,
    "origin": "user_upload",
    "mime_type": "application/pdf",
    "filename": "spec.pdf",
    "caption": "Latest draft",
    "size_bytes": 48213,
    "width": null,
    "height": null,
    "duration_ms": null,
    "created_at": "2026-04-03T10:20:00Z"
  }
}
```

`POST /mobile/v1/pairings`

```json
{
  "server_base_url": "https://orka.example.com"
}
```

```json
{
  "pairing_id": "0195f63a-48e0-7ce7-a2fc-529f6d740f95",
  "pairing_secret": "N6Toj1g0Tyh0oIq8v0f9pafKjW4G2P1fv5vD7ymVGjQ",
  "expires_at": "2026-04-01T12:10:00Z",
  "pairing_uri": "mobileorka://pair?server=https%3A%2F%2Forka.example.com&pairing_id=0195f63a-48e0-7ce7-a2fc-529f6d740f95&pairing_secret=N6Toj1g0Tyh0oIq8v0f9pafKjW4G2P1fv5vD7ymVGjQ"
}
```

`POST /mobile/v1/pairings/complete`

```json
{
  "pairing_id": "0195f63a-48e0-7ce7-a2fc-529f6d740f95",
  "pairing_secret": "N6Toj1g0Tyh0oIq8v0f9pafKjW4G2P1fv5vD7ymVGjQ",
  "device_id": "orka-localsession-123",
  "device_name": "Pixel 9",
  "platform": "android"
}
```

```json
{
  "access_token": "<jwt>",
  "access_token_expires_at": "2026-04-01T12:20:00Z",
  "refresh_token": "<opaque-refresh-token>",
  "refresh_token_expires_at": "2026-05-01T12:05:00Z",
  "user_id": "operator-1"
}
```
