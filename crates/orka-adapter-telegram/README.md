# orka-adapter-telegram

Telegram Bot API adapter for the Orka agent orchestration framework. Supports long-polling
and webhook ingestion modes, inbound media handling, slash commands, callback queries, and
outbound message editing.

## Modules

| Module       | Contents                                                             |
| ------------ | -------------------------------------------------------------------- |
| `lib.rs`     | `TelegramAdapter` — `ChannelAdapter` impl, outbound send logic       |
| `types.rs`   | Telegram Bot API deserialization types (`Update`, `TelegramMessage`) |
| `api.rs`     | `TelegramApi` — thin async HTTP client for the Bot API               |
| `media.rs`   | Inbound media resolution and outbound send-method selection          |
| `polling.rs` | Long-polling loop, message and callback-query processing             |
| `webhook.rs` | Axum-based webhook HTTP server                                       |

## Configuration

```toml
[adapters.telegram]
bot_token_secret = "adapters/telegram"   # Redis secret path (or use TELEGRAM_BOT_TOKEN env)
workspace = "default"
mode = "polling"                          # "polling" (default) or "webhook"
webhook_url = "https://example.com/telegram/webhook"
webhook_port = 8443
parse_mode = "HTML"                       # "HTML" (default), "MarkdownV2", "none"
streaming = false
```

## Authorization

The current adapter configuration does not expose a per-user allowlist. The bot is open to
any Telegram user who can reach it. If access control is required, enforce it externally
at the deployment boundary or reintroduce an explicit ACL field in `TelegramAdapterConfig`
before documenting it.

## Inbound metadata keys

These keys are set on every `Envelope` received from Telegram:

| Key                          | Type    | Description                                       |
| ---------------------------- | ------- | ------------------------------------------------- |
| `telegram_chat_id`           | integer | Telegram chat ID                                  |
| `telegram_message_id`        | integer | Message ID within the chat                        |
| `telegram_user_id`           | integer | Sender's Telegram user ID                         |
| `telegram_user_name`         | string  | Sender's display name (first + last)              |
| `telegram_username`          | string  | Sender's `@username` (if set)                     |
| `telegram_message_thread_id` | integer | Forum thread ID (supergroups only)                |
| `telegram_edited`            | bool    | Present and `true` for edited messages            |
| `telegram_callback_query_id` | string  | Present on callback-query envelopes               |
| `chat_type`                  | string  | `"direct"` for private chats, `"group"` otherwise |

## Outbound metadata keys

Set these keys on `OutboundMessage.metadata` to control send behaviour:

| Key                          | Type    | Description                                                 |
| ---------------------------- | ------- | ----------------------------------------------------------- |
| `telegram_chat_id`           | integer | **Required.** Destination chat                              |
| `telegram_message_id`        | integer | If present, sets reply-to on the sent message               |
| `telegram_message_thread_id` | integer | If present, sends into the specified forum thread           |
| `telegram_parse_mode`        | string  | Per-message parse mode override                             |
| `telegram_inline_keyboard`   | array   | Inline keyboard markup (array of button rows)               |
| `telegram_edit_message_id`   | integer | If present, edits that message instead of sending a new one |
