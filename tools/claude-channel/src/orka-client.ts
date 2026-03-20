import WebSocket from "ws";
import { config } from "./config.js";

/** Shape of the JSON Orka sends over the WebSocket for a completed outbound message. */
export interface OutboundMessage {
  channel: string;
  session_id: string;
  payload: {
    type: string;
    text?: string;
    [key: string]: unknown;
  };
  reply_to?: string;
  metadata: Record<string, unknown>;
}

export type MessageHandler = (msg: OutboundMessage) => void;

/** Client that bridges Orka's Custom Adapter over HTTP and WebSocket. */
export class OrkaClient {
  private ws: WebSocket | null = null;
  private handlers: MessageHandler[] = [];
  private reconnectTimer: ReturnType<typeof setTimeout> | null = null;
  private closed = false;

  /** Open the WebSocket connection; auto-reconnects on disconnect. */
  connect(): void {
    const wsUrl = config.orkaUrl.replace(/^http/, "ws");
    const url = `${wsUrl}/api/v1/ws?session_id=${config.sessionId}`;

    this.ws = new WebSocket(url);

    this.ws.on("open", () => {
      console.error(
        `[orka-channel] WebSocket connected — session: ${config.sessionId}`,
      );
    });

    this.ws.on("message", (data: Buffer) => {
      try {
        const msg = JSON.parse(data.toString()) as Record<string, unknown>;

        // Discard stream-chunk frames (they have no top-level `channel` string field).
        if (typeof msg.channel !== "string") return;

        const outbound = msg as unknown as OutboundMessage;

        // Apply channel filter.
        if (config.channels !== "*") {
          const allowed = config.channels.split(",").map((c) => c.trim());
          if (!allowed.includes(outbound.channel)) return;
        }

        for (const handler of this.handlers) handler(outbound);
      } catch {
        // Ignore non-JSON or malformed frames.
      }
    });

    this.ws.on("close", () => {
      if (this.closed) return;
      console.error("[orka-channel] WebSocket closed — reconnecting in 5 s…");
      this.reconnectTimer = setTimeout(() => this.connect(), 5_000);
    });

    this.ws.on("error", (err: Error) => {
      console.error("[orka-channel] WebSocket error:", err.message);
    });
  }

  /** Register a handler called for every qualifying outbound message. */
  onMessage(handler: MessageHandler): void {
    this.handlers.push(handler);
  }

  /**
   * Send a reply back into Orka via the Custom Adapter's inbound HTTP endpoint.
   * @param sessionId  Orka session to target.
   * @param text       Reply text.
   * @param channel    Optional source channel hint stored in metadata.
   * @param metadata   Extra key-value pairs merged into the message metadata.
   */
  async sendMessage(
    sessionId: string,
    text: string,
    channel?: string,
    metadata?: Record<string, unknown>,
  ): Promise<{ message_id: string; session_id: string }> {
    const body: Record<string, unknown> = { session_id: sessionId, text };

    const merged: Record<string, unknown> = { ...(metadata ?? {}) };
    if (channel) merged.source_channel = channel;
    if (Object.keys(merged).length > 0) body.metadata = merged;

    const res = await fetch(`${config.orkaUrl}/api/v1/message`, {
      method: "POST",
      headers: { "Content-Type": "application/json" },
      body: JSON.stringify(body),
    });

    if (!res.ok) {
      throw new Error(
        `Orka /api/v1/message returned HTTP ${res.status}: ${await res.text()}`,
      );
    }
    return res.json() as Promise<{ message_id: string; session_id: string }>;
  }

  /** List active sessions from the main Orka server. */
  async listSessions(limit = 20): Promise<unknown> {
    const res = await fetch(
      `${config.orkaApiUrl}/api/v1/sessions?limit=${limit}`,
    );
    if (!res.ok) {
      throw new Error(
        `Orka /api/v1/sessions returned HTTP ${res.status}: ${await res.text()}`,
      );
    }
    return res.json();
  }

  /** Fetch details for a single session from the main Orka server. */
  async getSession(sessionId: string): Promise<unknown> {
    const res = await fetch(
      `${config.orkaApiUrl}/api/v1/sessions/${sessionId}`,
    );
    if (!res.ok) {
      throw new Error(
        `Orka /api/v1/sessions/${sessionId} returned HTTP ${res.status}: ${await res.text()}`,
      );
    }
    return res.json();
  }

  /** Close the WebSocket and stop reconnection attempts. */
  disconnect(): void {
    this.closed = true;
    if (this.reconnectTimer) clearTimeout(this.reconnectTimer);
    this.ws?.close();
  }
}
