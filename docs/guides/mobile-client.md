# Mobile Client Guide

The mobile app should use Orka's dedicated product-facing API, not the
management routes and not the legacy `custom` adapter surface.

## Auth Model

- Mobile routes are exposed only when JWT auth is configured.
- The authenticated JWT subject (`sub`) becomes the owning `user_id`.
- The mobile client must send `Authorization: Bearer <token>`.

## Endpoints

Base path: `/mobile/v1`

| Method | Path | Purpose |
| --- | --- | --- |
| `GET` | `/me` | Return the authenticated user profile (`user_id`, `scopes`) |
| `GET` | `/conversations` | List recent conversations for the current user |
| `POST` | `/conversations` | Create an empty conversation |
| `GET` | `/conversations/{id}` | Fetch conversation metadata |
| `GET` | `/conversations/{id}/messages` | Fetch transcript messages |
| `POST` | `/conversations/{id}/messages` | Append a user message and enqueue Orka processing |
| `GET` | `/conversations/{id}/stream` | Stream assistant progress and completion events as SSE |

## Streaming Contract

The stream endpoint emits Server-Sent Events with these event names:

- `message_delta`
  Data: `{ "conversation_id", "reply_to"?, "delta" }`
- `message_completed`
  Data: `{ "message": ConversationMessage }`
- `message_failed`
  Data: `{ "conversation_id", "error" }`
- `stream_done`
  Data: `{ "conversation_id" }`

`message_delta` carries incremental assistant text while the model is
responding. `message_completed` is emitted only after the assistant message has
been persisted to the conversation transcript.

## Conversation Model

- `Conversation` is the product-facing chat thread exposed to clients.
- `Session` remains the runtime orchestration concept used internally by Orka.
- In the current implementation, each mobile conversation has a 1:1 backing
  runtime session and shares the same underlying UUID value, but the API
  models are intentionally distinct.
