#!/usr/bin/env node
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { formatEnvelope, resolveCliPath, runCli } from "./cli.js";

const server = new McpServer({
  name: "wiparse",
  version: "1.0.1",
});

function textResult(envelope: Awaited<ReturnType<typeof runCli>>) {
  return {
    content: [{ type: "text" as const, text: formatEnvelope(envelope) }],
  };
}

async function withCli(args: string[], options?: Parameters<typeof runCli>[1]) {
  return textResult(await runCli(args, options));
}

server.tool(
  "wiparse_cli_info",
  "Return resolved WiParse CLI path and supported command groups.",
  {},
  async () => ({
    content: [
      {
        type: "text",
        text: JSON.stringify(
          {
            cli_path: resolveCliPath(),
            version_tool: "wiparse_version",
            groups: [
              "serial (read, send)",
              "parse (line, metrics, file)",
              "session (list, show)",
              "wave (live, session, export)",
              "scope (list, shot, wave)",
            ],
            docs: "docs/CLI_REFERENCE.md",
            note: "Instrument workbench (VISA multi-instrument) is GUI-only; CLI scope commands target Tektronix/VISA oscilloscopes.",
          },
          null,
          2,
        ),
      },
    ],
  }),
);

server.tool("wiparse_version", "Get WiParse CLI version.", {}, async () =>
  withCli(["version"]),
);

server.tool("wiparse_ports", "List available serial ports.", {}, async () =>
  withCli(["ports"]),
);

server.tool(
  "wiparse_serial_read",
  "Read metrics and logs from a serial port for a bounded duration or count.",
  {
    port: z.string().describe("Serial port name, e.g. COM3"),
    baud: z.number().int().positive().optional().default(2_000_000),
    duration_sec: z.number().positive().optional(),
    max_metrics: z.number().int().positive().optional(),
    max_logs: z.number().int().positive().optional(),
    demo: z.boolean().optional().default(false),
    save_db: z.boolean().optional().default(false),
    config: z.string().optional().describe("Optional WCM config file path"),
  },
  async ({ port, baud, duration_sec, max_metrics, max_logs, demo, save_db, config }) => {
    const args = ["serial", "read", "--port", port, "--baud", String(baud)];
    if (duration_sec !== undefined) {
      args.push("--duration", String(duration_sec));
    }
    if (max_metrics !== undefined) {
      args.push("--max-metrics", String(max_metrics));
    }
    if (max_logs !== undefined) {
      args.push("--max-logs", String(max_logs));
    }
    if (demo) args.push("--demo");
    if (save_db) args.push("--save-db");
    return withCli(args, { config, timeoutMs: duration_sec ? duration_sec * 1000 + 30_000 : 120_000 });
  },
);

server.tool(
  "wiparse_serial_send",
  "Send hex bytes to a serial port.",
  {
    port: z.string(),
    hex: z.string().describe("Hex string without spaces, e.g. AA55FF"),
    baud: z.number().int().positive().optional().default(2_000_000),
    config: z.string().optional(),
  },
  async ({ port, hex, baud, config }) =>
    withCli(
      ["serial", "send", "--port", port, "--baud", String(baud), "--hex", hex],
      { config },
    ),
);

server.tool(
  "wiparse_parse_qi_line",
  "Parse a Qi wireless charging ASK/FSK log line.",
  { text: z.string() },
  async ({ text }) => withCli(["parse", "line", "--text", text]),
);

server.tool(
  "wiparse_parse_metrics",
  "Parse an AA55 metrics frame string.",
  { text: z.string() },
  async ({ text }) => withCli(["parse", "metrics", "--text", text]),
);

server.tool(
  "wiparse_parse_file",
  "Parse Qi lines and metrics frames from a log file.",
  {
    path: z.string(),
    limit: z.number().int().positive().optional(),
    config: z.string().optional(),
  },
  async ({ path, limit, config }) => {
    const args = ["parse", "file", "--path", path];
    if (limit !== undefined) args.push("--limit", String(limit));
    return withCli(args, { config });
  },
);

server.tool(
  "wiparse_session_list",
  "List saved SQLite capture sessions.",
  {
    limit: z.number().int().positive().optional().default(20),
    config: z.string().optional(),
  },
  async ({ limit, config }) =>
    withCli(["session", "list", "--limit", String(limit)], { config }),
);

server.tool(
  "wiparse_session_show",
  "Show metadata and row counts for one session.",
  {
    session_id: z.number().int().positive(),
    config: z.string().optional(),
  },
  async ({ session_id, config }) =>
    withCli(["session", "show", "--id", String(session_id)], { config }),
);

server.tool(
  "wiparse_wave_live",
  "Capture live metrics from serial and return waveform JSON.",
  {
    port: z.string(),
    baud: z.number().int().positive().optional().default(2_000_000),
    duration_sec: z.number().positive().optional().default(5),
    channels: z
      .string()
      .optional()
      .default("v_in,i_in,v_out,i_out,p"),
    demo: z.boolean().optional().default(false),
    config: z.string().optional(),
  },
  async ({ port, baud, duration_sec, channels, demo, config }) => {
    const args = [
      "wave",
      "live",
      "--port",
      port,
      "--baud",
      String(baud),
      "--duration",
      String(duration_sec),
      "--channels",
      channels,
    ];
    if (demo) args.push("--demo");
    return withCli(args, {
      config,
      timeoutMs: duration_sec * 1000 + 30_000,
    });
  },
);

server.tool(
  "wiparse_wave_session",
  "Build waveform JSON or CSV from a saved session.",
  {
    session_id: z.number().int().positive(),
    from_sec: z.number().optional(),
    to_sec: z.number().optional(),
    channels: z
      .string()
      .optional()
      .default("v_in,i_in,v_out,i_out,v_bat,i_bat,p"),
    format: z.enum(["json", "csv"]).optional().default("json"),
    config: z.string().optional(),
  },
  async ({ session_id, from_sec, to_sec, channels, format, config }) => {
    const args = [
      "wave",
      "session",
      "--session-id",
      String(session_id),
      "--channels",
      channels,
      "--format",
      format,
    ];
    if (from_sec !== undefined) args.push("--from", String(from_sec));
    if (to_sec !== undefined) args.push("--to", String(to_sec));
    return withCli(args, { config });
  },
);

server.tool(
  "wiparse_wave_export",
  "Export session metrics to CSV or JSON file.",
  {
    session_id: z.number().int().positive(),
    out: z.string(),
    format: z.enum(["csv", "json", "jsonl"]).optional().default("csv"),
    from_sec: z.number().optional(),
    to_sec: z.number().optional(),
    config: z.string().optional(),
  },
  async ({ session_id, out, format, from_sec, to_sec, config }) => {
    const args = [
      "wave",
      "export",
      "--session-id",
      String(session_id),
      "--format",
      format,
      "--out",
      out,
    ];
    if (from_sec !== undefined) args.push("--from", String(from_sec));
    if (to_sec !== undefined) args.push("--to", String(to_sec));
    return withCli(args, { config });
  },
);

server.tool(
  "wiparse_scope_list",
  "List connected VISA oscilloscopes and capabilities.",
  { config: z.string().optional() },
  async ({ config }) => withCli(["scope", "list"], { config }),
);

server.tool(
  "wiparse_scope_shot",
  "Capture oscilloscope screenshot to PNG.",
  {
    index: z.number().int().nonnegative().optional().default(0),
    out: z.string().optional(),
    config: z.string().optional(),
  },
  async ({ index, out, config }) => {
    const args = ["scope", "shot", "--index", String(index)];
    if (out) args.push("--out", out);
    return withCli(args, { config, timeoutMs: 180_000 });
  },
);

server.tool(
  "wiparse_scope_waveform",
  "Read oscilloscope waveform trace from a channel.",
  {
    index: z.number().int().nonnegative().optional().default(0),
    channel: z.string().optional().default("CH1"),
    points: z.number().int().positive().optional(),
    config: z.string().optional(),
  },
  async ({ index, channel, points, config }) => {
    const args = ["scope", "wave", "--index", String(index), "--channel", channel];
    if (points !== undefined) args.push("--points", String(points));
    return withCli(args, { config, timeoutMs: 180_000 });
  },
);

async function main() {
  const transport = new StdioServerTransport();
  await server.connect(transport);
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
