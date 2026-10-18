#!/usr/bin/env node
import { McpServer } from "@modelcontextprotocol/sdk/server/mcp.js";
import { StdioServerTransport } from "@modelcontextprotocol/sdk/server/stdio.js";
import { z } from "zod";
import { createRequire } from "node:module";
import { apiHealth, apiInvoke, defaultApiUrl } from "./http.js";
const packageVersion = createRequire(import.meta.url)("../package.json").version;
const server = new McpServer({
    name: "wiparse",
    version: packageVersion,
});
function compact(body) {
    return {
        content: [{ type: "text", text: JSON.stringify(body) }],
    };
}
async function invoke(method, params = {}) {
    try {
        return compact(await apiInvoke(method, params));
    }
    catch (e) {
        return compact({
            ok: false,
            error: { message: e instanceof Error ? e.message : String(e) },
        });
    }
}
server.tool("wiparse_brief", "WiParse compact live brief (phase, counts, alerts, notables). Poll this; do not read serial files.", {
    since_row: z.number().int().nonnegative().optional().default(0),
    detail: z.enum(["qi", "m", "alerts"]).optional(),
}, async ({ since_row, detail }) => {
    try {
        await apiHealth();
    }
    catch (e) {
        return compact({
            ok: false,
            error: { message: e instanceof Error ? e.message : "GUI API down" },
            hint: `start WiParse.exe; WIPARSE_URL=${defaultApiUrl()}`,
        });
    }
    return invoke("log.brief", { since_row, detail });
});
server.tool("wiparse_test", "Closed-loop test on the GUI. start needs a JSON plan; status/abort/pack for the run.", {
    action: z.enum(["start", "status", "abort", "pack"]),
    plan: z.record(z.unknown()).optional().describe("TestPlan JSON for start"),
    port: z.string().optional(),
    baud: z.number().int().positive().optional(),
    reason: z.string().optional(),
}, async ({ action, plan, port, baud, reason }) => {
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
});
server.tool("wiparse_select", "Set GUI serial port/baud without opening. Stop the monitor first if it is running.", {
    port: z.string().optional(),
    baud: z.number().int().positive().optional(),
}, async ({ port, baud }) => {
    if (!port && baud == null) {
        return compact({ ok: false, error: { message: "port or baud required" } });
    }
    return invoke("serial.select", { port, baud });
});
server.tool("wiparse_send", "Queue hex on the GUI serial monitor (prefer plan macros over ad-hoc hex).", { hex: z.string().describe("Hex bytes, e.g. AA55") }, async ({ hex }) => invoke("serial.send", { hex }));
server.tool("wiparse_report_pack", "Evidence-pack summary only (paths + brief + correlate). Write the report from this; do not open serial.txt.", {}, async () => invoke("test.pack", {}));
const UI_METHODS = {
    state: "ui.state",
    show: "ui.show",
    panels: "ui.panels",
    prefs: "ui.prefs",
    "serial.open": "ui.serial.open",
    "serial.close": "ui.serial.close",
    "serial.clear": "ui.serial.clear",
    "serial.filter": "ui.serial.filter",
    "serial.tab": "ui.serial.tab",
    "serial.name": "ui.serial.name",
    "serial.browser": "ui.serial.browser",
    "wave.open": "ui.wave.open",
    "wave.close": "ui.wave.close",
    "wave.select": "ui.wave.select",
    "wave.browser": "ui.wave.browser",
    "wave.bus": "ui.wave.bus",
    "wave.cursor": "ui.wave.cursor",
    "wave.fit": "ui.wave.fit",
    "calc.get": "ui.calc.get",
    "calc.set": "ui.calc.set",
    "instrument.select": "ui.instrument.select",
    "instrument.scan": "instrument.scan",
    "instrument.list": "instrument.list",
    "instrument.overview": "instrument.overview",
    "instrument.connect": "instrument.connect",
    "instrument.connect_all": "instrument.connect_all",
    "instrument.disconnect": "instrument.disconnect",
    "instrument.measure": "instrument.measure",
    "instrument.capture": "instrument.capture",
    "instrument.waveform": "instrument.waveform",
    "instrument.waveform_source": "instrument.waveform_source",
    "instrument.command": "instrument.command",
    "report.open": "ui.report.open",
    "report.browser": "ui.report.browser",
    "project.list": "ui.project.list",
    "project.select": "ui.project.select",
    "project.create": "ui.project.create",
    "project.add_test": "ui.project.add_test",
    "project.remove_test": "ui.project.remove_test",
    "test_tool.run": "ui.test_tool.run",
    "test_tool.stop": "ui.test_tool.stop",
    "test_tool.config.validate": "ui.test_tool.config.validate",
    "test_tool.config.list": "ui.test_tool.config.list",
    "test_tool.config.get": "ui.test_tool.config.get",
    "test_tool.config.create": "ui.test_tool.config.create",
    "test_tool.config.save": "ui.test_tool.config.save",
    "test_tool.config.duplicate": "ui.test_tool.config.duplicate",
    "test_tool.config.archive": "ui.test_tool.config.archive",
    "test_tool.config.restore": "ui.test_tool.config.restore",
    "test_tool.config.import": "ui.test_tool.config.import",
    "test_tool.config.export": "ui.test_tool.config.export",
    "test_tool.config.batch": "ui.test_tool.config.batch",
    "test_tool.config.archives": "ui.test_tool.config.archives",
    "test_tool.project.get": "ui.test_tool.project.get",
    "test_tool.project.save": "ui.test_tool.project.save",
    "test_tool.project.validate": "ui.test_tool.project.validate",
    "test_tool.project.migration.preview": "ui.test_tool.project.migration.preview",
    "test_tool.project.migration.commit": "ui.test_tool.project.migration.commit",
    "test_tool.result.get": "ui.test_tool.result.get",
    "test_tool.matrix.get": "ui.test_tool.matrix.get",
    "test_tool.preflight.state": "ui.test_tool.preflight.state",
    "test_tool.browser": "ui.test_tool.browser",
};
server.tool("wiparse_ui", "Drive WiParse.exe: tabs, panels, serial/wave/calc, instruments, Testing Hub project, and Test Report. Poll instrument.overview for device inventory and latest readings. report.open previews Markdown, JSON/JSONL, images, or embedded waveforms (no file body). project.* shares apps.project.active_id with Hub. instrument.command accepts Probe*/Bridge*. Use GUI 1.1.31+ for the updated report viewer.", {
    op: z.enum([
        "state",
        "show",
        "panels",
        "prefs",
        "serial.open",
        "serial.close",
        "serial.clear",
        "serial.filter",
        "serial.tab",
        "serial.name",
        "serial.browser",
        "wave.open",
        "wave.close",
        "wave.select",
        "wave.browser",
        "wave.bus",
        "wave.cursor",
        "wave.fit",
        "calc.get",
        "calc.set",
        "instrument.select",
        "instrument.scan",
        "instrument.list",
        "instrument.overview",
        "instrument.connect",
        "instrument.connect_all",
        "instrument.disconnect",
        "instrument.measure",
        "instrument.capture",
        "instrument.waveform",
        "instrument.waveform_source",
        "instrument.command",
        "report.open",
        "report.browser",
        "project.list",
        "project.select",
        "project.create",
        "project.add_test",
        "project.remove_test",
        "test_tool.run",
        "test_tool.stop",
        "test_tool.config.validate",
        "test_tool.config.list",
        "test_tool.config.get",
        "test_tool.config.create",
        "test_tool.config.save",
        "test_tool.config.duplicate",
        "test_tool.config.archive",
        "test_tool.config.restore",
        "test_tool.config.import",
        "test_tool.config.export",
        "test_tool.config.batch",
        "test_tool.config.archives",
        "test_tool.project.get",
        "test_tool.project.save",
        "test_tool.project.validate",
        "test_tool.project.migration.preview",
        "test_tool.project.migration.commit",
        "test_tool.result.get",
        "test_tool.matrix.get",
        "test_tool.preflight.state",
        "test_tool.browser",
    ]),
    tab: z
        .enum([
        "serial",
        "calculator",
        "instruments",
        "waveform",
        "data_analysis",
        "test_report",
        "test_tool",
    ])
        .optional()
        .describe("For op=show; also accepted inside params.tab"),
    params: z
        .record(z.unknown())
        .optional()
        .describe("Op params. instrument.overview: {}. instrument.command: {device_id, command}. project.list: {}. project.select/create: {id} (+ name). project.add_test/remove_test: {id}. report.open: {path, view?, overlay?}; omit view for automatic file type; JSON/JSONL opens in the report viewer. Path must be under the active project; never dumps file bodies. test_tool.run: {id, preflight?, params?} (params overlay Hub form, e.g. {port, triggers}). test_tool.stop: {}."),
}, async ({ op, tab, params }) => {
    const method = UI_METHODS[op];
    const payload = { ...(params ?? {}) };
    if (tab != null && payload.tab == null) {
        payload.tab = tab;
    }
    return invoke(method, payload);
});
async function main() {
    const transport = new StdioServerTransport();
    await server.connect(transport);
}
main().catch((error) => {
    console.error(error);
    process.exit(1);
});
