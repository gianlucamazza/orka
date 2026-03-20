/**
 * Configuration loaded from environment variables.
 *
 * ORKA_URL        - Custom Adapter base URL (HTTP/WS), used for POST /api/v1/message
 *                   and WS /api/v1/ws. Default: http://127.0.0.1:8081
 * ORKA_API_URL    - Main Orka server URL, used for /api/v1/sessions endpoints.
 *                   Default: http://127.0.0.1:8080
 * ORKA_SESSION_ID - UUID of the Orka session to subscribe to on the WS.
 *                   Default: auto-generated UUID (new session per process start).
 * ORKA_CHANNELS   - Comma-separated list of source channels to forward to Claude Code.
 *                   Use '*' to forward all. Default: *
 */
export const config = {
  orkaUrl: process.env.ORKA_URL ?? "http://127.0.0.1:8081",
  orkaApiUrl: process.env.ORKA_API_URL ?? "http://127.0.0.1:8080",
  sessionId: process.env.ORKA_SESSION_ID ?? crypto.randomUUID(),
  channels: process.env.ORKA_CHANNELS ?? "*",
} as const;
