#!/usr/bin/env bun
/**
 * orka-claude-channel — MCP server that bridges Orka messages into a Claude Code session.
 *
 * Inbound flow  (Orka → Claude Code):
 *   1. Connects to Orka's Custom Adapter WebSocket (/api/v1/ws?session_id=…).
 *   2. Every outbound message received is pushed to Claude Code as a
 *      `notifications/message` channel notification.
 *
 * Outbound flow (Claude Code → Orka):
 *   1. Claude calls the `reply` tool with session_id + text.
 *   2. The plugin POSTs to /api/v1/message on the Custom Adapter.
 *   3. Orka routes the message back to the originating chat platform.
 */

import { Server } from "@modelcontextprotocol/sdk/server/index.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import {
  CallToolRequestSchema,
  ListToolsRequestSchema,
} from "@modelcontextprotocol/sdk/types.js";
import { z } from "zod";
import { config } from "./config.js";
import { OrkaClient } from "./orka-client.js";

// ---------------------------------------------------------------------------
// MCP Server
// ---------------------------------------------------------------------------

const server = new Server(
  { name: "orka", version: "0.1.0" },
  {
    capabilities: {
      // Opt-in to the experimental Claude Code channel protocol.
      experimental: { "claude/channel": {} },
      tools: {},
    },
    instructions: `\
You are connected to the Orka AI agent orchestration framework.

Messages from users on chat platforms (Telegram, Discord, Slack, WhatsApp) arrive as
<channel source="orka" channel="<platform>" session_id="<uuid>"> notifications.

Use the available tools to interact with Orka:
- reply          — send a response back to a user on their platform
- list_sessions  — see all active Orka sessions
- session_info   — inspect a specific session`,
  },
);

// ---------------------------------------------------------------------------
// Orka WebSocket client
// ---------------------------------------------------------------------------

const orka = new OrkaClient();

// Forward every qualifying Orka outbound message to Claude Code as a channel notification.
orka.onMessage((msg) => {
  const text = msg.payload.text ?? JSON.stringify(msg.payload);

  const channelXml = [
    `<channel source="orka" channel="${msg.channel}" session_id="${msg.session_id}">`,
    text,
    `</channel>`,
  ].join("\n");

  // Push the event into the active Claude Code session.
  server.notification({
    method: "notifications/message",
    params: {
      role: "user",
      content: { type: "text", text: channelXml },
    },
  });
});

// ---------------------------------------------------------------------------
// Tool definitions
// ---------------------------------------------------------------------------

const ReplyInput = z.object({
  session_id: z.string().uuid().describe("Orka session ID to reply to"),
  text: z.string().min(1).describe("Reply text to send"),
  channel: z
    .string()
    .optional()
    .describe("Target channel hint (e.g. telegram, discord)"),
  metadata: z
    .record(z.string(), z.unknown())
    .optional()
    .describe("Extra metadata forwarded to the adapter"),
});

const SessionInfoInput = z.object({
  session_id: z.string().uuid().describe("Orka session ID to inspect"),
});

server.setRequestHandler(ListToolsRequestSchema, async () => ({
  tools: [
    {
      name: "reply",
      description:
        "Send a reply to a user through Orka. Use session_id from the channel notification.",
      inputSchema: {
        type: "object",
        properties: {
          session_id: {
            type: "string",
            description: "Orka session ID to reply to",
          },
          text: { type: "string", description: "Reply text to send" },
          channel: {
            type: "string",
            description: "Target channel hint (e.g. telegram, discord)",
          },
          metadata: {
            type: "object",
            description: "Extra metadata forwarded to the adapter",
          },
        },
        required: ["session_id", "text"],
      },
    },
    {
      name: "list_sessions",
      description: "List active Orka sessions.",
      inputSchema: {
        type: "object",
        properties: {
          limit: {
            type: "number",
            description: "Maximum number of sessions to return (default 20)",
          },
        },
      },
    },
    {
      name: "session_info",
      description: "Get details for a specific Orka session.",
      inputSchema: {
        type: "object",
        properties: {
          session_id: {
            type: "string",
            description: "Orka session ID to inspect",
          },
        },
        required: ["session_id"],
      },
    },
  ],
}));

// ---------------------------------------------------------------------------
// Tool handlers
// ---------------------------------------------------------------------------

server.setRequestHandler(CallToolRequestSchema, async (req) => {
  const { name, arguments: args } = req.params;

  switch (name) {
    case "reply": {
      const input = ReplyInput.parse(args);
      const result = await orka.sendMessage(
        input.session_id,
        input.text,
        input.channel,
        input.metadata as Record<string, unknown> | undefined,
      );
      return {
        content: [
          {
            type: "text",
            text: `Message sent — message_id: ${result.message_id}, session_id: ${result.session_id}`,
          },
        ],
      };
    }

    case "list_sessions": {
      const limit =
        typeof (args as Record<string, unknown>)?.limit === "number"
          ? ((args as Record<string, unknown>).limit as number)
          : 20;
      const sessions = await orka.listSessions(limit);
      return {
        content: [{ type: "text", text: JSON.stringify(sessions, null, 2) }],
      };
    }

    case "session_info": {
      const input = SessionInfoInput.parse(args);
      const session = await orka.getSession(input.session_id);
      return {
        content: [{ type: "text", text: JSON.stringify(session, null, 2) }],
      };
    }

    default:
      throw new Error(`Unknown tool: ${name}`);
  }
});

// ---------------------------------------------------------------------------
// Start
// ---------------------------------------------------------------------------

async function main() {
  console.error(
    `[orka-channel] Starting — session: ${config.sessionId}, url: ${config.orkaUrl}`,
  );

  orka.connect();

  const transport = new StdioServerTransport();
  await server.connect(transport);

  // Disconnect WebSocket when the MCP transport closes.
  transport.onclose = () => {
    orka.disconnect();
  };
}

main().catch((err: unknown) => {
  console.error("[orka-channel] Fatal:", err);
  process.exit(1);
});
