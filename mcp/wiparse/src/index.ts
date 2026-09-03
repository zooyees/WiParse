#!/usr/bin/env node
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { apiHealth, apiInvoke, defaultApiUrl } from "./http.js";

const server = new McpServer({
  name: "wiparse",
  version: "1.1.3",
});

function compact(body: unknown) {
  return {
    content: [{ type: "text" as const, text: JSON.stringify(body) }],
  };
}

async function invoke(method: string, params: unknown = {}) {
  try {
    return compact(await apiInvoke(method, params));
  } catch (e) {
    return compact({
      ok: false,
      error: { message: e instanceof Error ? e.message : String(e) },
    });
  }
}

server.tool(
  "wiparse_brief",
  "WiParse compact live brief (phase, counts, alerts, notables). Poll this; do not read serial files.",
  {
    since_row: z.number().int().nonnegative().optional().default(0),
    detail: z.enum(["qi", "m", "alerts"]).optional(),
  },
  async ({ since_row, detail }) => {
    try {
      await apiHealth();
    } catch (e) {
      return compact({
        ok: false,
        error: { message: e instanceof Error ? e.message : "GUI API down" },
        hint: `start WiParse.exe; WIPARSE_URL=${defaultApiUrl()}`,
      });
    }
    return invoke("log.brief", { since_row, detail });
  },
);

server.tool(
  "wiparse_test",
  "Closed-loop test on the GUI. start needs a JSON plan; status/abort/pack for the run.",
  {
    action: z.enum(["start", "status", "abort", "pack"]),
    plan: z.record(z.unknown()).optional().describe("TestPlan JSON for start"),
    port: z.string().optional(),
    baud: z.number().int().positive().optional(),
    reason: z.string().optional(),
  },
  async ({ action, plan, port, baud, reason }) => {
    if (action === "start") {
      if (!plan) {
        return compact({ ok: false, error: { message: "start requires plan" } });
      }
      return invoke("test.start", { plan, port, baud });
    }
    if (action === "abort") {
      return invoke("test.abort", { reason: reason ?? "user" });
    }
    if (action === "pack") {
      return invoke("test.pack", {});
    }
    return invoke("test.status", {});
  },
);

server.tool(
  "wiparse_select",
  "Set GUI serial port/baud without opening. Stop the monitor first if it is running.",
  {
    port: z.string().optional(),
    baud: z.number().int().positive().optional(),
  },
  async ({ port, baud }) => {
    if (!port && baud == null) {
      return compact({ ok: false, error: { message: "port or baud required" } });
    }
    return invoke("serial.select", { port, baud });
  },
);

server.tool(
  "wiparse_send",
  "Queue hex on the GUI serial monitor (prefer plan macros over ad-hoc hex).",
  { hex: z.string().describe("Hex bytes, e.g. AA55") },
  async ({ hex }) => invoke("serial.send", { hex }),
);

server.tool(
  "wiparse_report_pack",
  "Evidence-pack summary only (paths + brief + correlate). Write the report from this; do not open serial.txt.",
  {},
  async () => invoke("test.pack", {}),
);

async function main() {
  const transport = new StdioServerTransport();
  await server.connect(transport);
}

main().catch((error) => {
  console.error(error);
  process.exit(1);
});
