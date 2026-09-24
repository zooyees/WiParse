import test from "node:test";
import assert from "node:assert/strict";
import {
  pickInstrument,
  suggestedParamsFromInstrument,
  validatePluginManifest,
  validateStationConfig,
  applyParamPaths,
  expandTemplates,
  colocateStatusWithIsf,
  loadOverlayFile,
  mergeArgs,
  resolveProjectLayout,
  injectDefaultPathTemplates,
  normalizeArtifacts,
  makeStamp,
  writeRunJson,
  ensureTestInProject,
  loadStationConfig,
  sanitizeTestId,
  resolveReportName,
} from "./plugin-contract.mjs";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const devices = [
  {
    device_id: 1,
    kind: "dc_source",
    resource: "USB0::PSU::INSTR",
    identity: { model: "E36312A" },
  },
  {
    device_id: 2,
    kind: "oscilloscope",
    resource: "USB0::SCOPE::INSTR",
    identity: { model: "MDO3014" },
  },
];

test("pickInstrument prefers resource then kind", () => {
  const byRes = pickInstrument(devices, {
    kind: "oscilloscope",
    resource: "USB0::SCOPE::INSTR",
    deviceId: 1,
  });
  assert.equal(byRes.device_id, 2);

  const byKind = pickInstrument(devices, { kind: "oscilloscope" });
  assert.equal(byKind.identity.model, "MDO3014");

  const psu = pickInstrument(devices, { kind: "psu" });
  assert.equal(psu.kind, "dc_source");
});

test("suggestedParamsFromInstrument maps binds", () => {
  const p = suggestedParamsFromInstrument(devices[1], {
    prefer_device_id: "device_id",
    scope_resource: "resource",
    scope_model: "model",
    scope_kind: "kind",
  });
  assert.equal(p.prefer_device_id, "2");
  assert.equal(p.scope_model, "MDO3014");
  assert.equal(p.scope_kind, "oscilloscope");
});

test("scope-serial-monitor plugin.json validates", () => {
  const here = path.dirname(fileURLToPath(import.meta.url));
  const raw = JSON.parse(
    fs.readFileSync(
      path.join(here, "..", "plugins", "scope-serial-monitor", "plugin.json"),
      "utf8"
    )
  );
  const check = validatePluginManifest(raw);
  assert.equal(check.ok, true, check.errors?.join("; "));
  assert.equal(raw.version, "0.6.0");
  assert.equal(raw.engines?.wiparse, ">=1.1.16");
  assert.ok(!String(raw.description || "").includes("tests/scope-serial-monitor"));
  const station = JSON.parse(
    fs.readFileSync(
      path.join(here, "..", "plugins", "scope-serial-monitor", "station.json"),
      "utf8"
    )
  );
  const stationCheck = validateStationConfig(station);
  assert.equal(stationCheck.ok, true, stationCheck.errors?.join("; "));
  assert.equal(station.gui?.min_version, "1.1.16");
  assert.ok(String(station.paths?.test_dir || "").includes("{test_id}"));
  assert.equal(raw.params.some((p) => p.type === "device"), true);
  assert.equal(raw.params.some((p) => p.type === "json" && p.name === "triggers"), true);
  assert.ok(Array.isArray(raw.outputs) && raw.outputs.some((o) => o.id === "waves"));
  assert.equal(raw.report_name, "ScopeSerial");
});

test("all bundled plugins declare report_name and 1.1.16 engines", () => {
  const here = path.dirname(fileURLToPath(import.meta.url));
  const root = path.join(here, "..", "plugins");
  const ids = fs.readdirSync(root).filter((id) => fs.existsSync(path.join(root, id, "plugin.json")));
  assert.ok(ids.length >= 6, ids.join(","));
  for (const id of ids) {
    const raw = JSON.parse(fs.readFileSync(path.join(root, id, "plugin.json"), "utf8"));
    const check = validatePluginManifest(raw);
    assert.equal(check.ok, true, `${id}: ${check.errors?.join("; ")}`);
    assert.ok(String(raw.report_name || "").trim(), `${id} missing report_name`);
    assert.notEqual(raw.report_name, raw.name, `${id} report_name must not copy marketing name`);
    assert.equal(raw.engines?.wiparse, ">=1.1.16", id);
    assert.ok(Array.isArray(raw.outputs) && raw.outputs.length >= 1, `${id} outputs[]`);
    assert.ok(raw.params?.some((p) => p.name === "file_prefix"), `${id} file_prefix param`);
    const stationPath = path.join(root, id, "station.json");
    if (fs.existsSync(stationPath)) {
      const station = JSON.parse(fs.readFileSync(stationPath, "utf8"));
      const s = validateStationConfig(station);
      assert.equal(s.ok, true, `${id} station: ${s.errors?.join("; ")}`);
      assert.equal(station.gui?.min_version, "1.1.16", id);
    }
  }
});

test("discoverPlugins keeps report_name and outputs", async () => {
  const { discoverPlugins } = await import("../runner.mjs");
  const here = path.dirname(fileURLToPath(import.meta.url));
  const plugins = discoverPlugins(path.join(here, "..", "plugins"));
  const smoke = plugins.find((p) => p.id === "example-smoke");
  assert.ok(smoke, "example-smoke");
  assert.equal(smoke.report_name, "ExampleSmoke");
  assert.ok(Array.isArray(smoke.outputs) && smoke.outputs.length >= 1);
});

test("applyParamPaths parses json overlay onto station", () => {
  const station = {
    gui: { api: "http://127.0.0.1:7878" },
    paths: { file_prefix: "x", isf_dir: "a", report_dir: "b" },
    serial_triggers: { rising_edge: true, items: [] },
  };
  const defs = [
    { name: "triggers", type: "json", path: "serial_triggers.items" },
    { name: "rising_edge", type: "boolean", path: "serial_triggers.rising_edge" },
  ];
  const cfg = applyParamPaths(station, defs, {
    triggers: '[{"id":"T","type":"contains","pattern":"HIT"}]',
    rising_edge: "false",
  });
  assert.equal(cfg.serial_triggers.rising_edge, false);
  assert.equal(cfg.serial_triggers.items[0].id, "T");
});

test("station without isf_dir validates", () => {
  const check = validateStationConfig({
    gui: { api: "http://127.0.0.1:7878" },
    paths: { stop_file: "{plugin_dir}/run.stop" },
  });
  assert.equal(check.ok, true, check.errors?.join("; "));
});

test("expandTemplates second pass substitutes {isf_dir}", () => {
  const cfg = expandTemplates(
    {
      station: { product: "P" },
      paths: {
        isf_dir: "{data_root}/waves/{product}",
        status_file: "{isf_dir}/_loop_status.json",
      },
    },
    { dataRoot: "/data", pluginDir: "/plug" }
  );
  assert.equal(cfg.paths.status_file.replaceAll("\\", "/"), "/data/waves/P/_loop_status.json");
});

test("colocateStatusWithIsf does not overwrite explicit status_file", () => {
  const cfg = {
    paths: {
      isf_dir: "/waves",
      status_file: "/elsewhere/hud.json",
    },
  };
  colocateStatusWithIsf(cfg);
  assert.equal(cfg.paths.status_file, "/elsewhere/hud.json");
});

test("colocateStatusWithIsf fills empty status_file next to isf when no test_dir", () => {
  const cfg = { paths: { isf_dir: "/waves" } };
  colocateStatusWithIsf(cfg);
  assert.equal(cfg.paths.status_file.replaceAll("\\", "/"), "/waves/_loop_status.json");
});

test("colocateStatusWithIsf prefers test_dir over isf_dir", () => {
  const cfg = { paths: { isf_dir: "/waves", test_dir: "/proj/tests/x" } };
  colocateStatusWithIsf(cfg);
  assert.equal(cfg.paths.status_file.replaceAll("\\", "/"), "/proj/tests/x/status.json");
});

test("loadOverlayFile + applyParamPaths is run-scoped", () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "wiparse-overlay-"));
  const file = path.join(dir, "overlay.json");
  fs.writeFileSync(
    file,
    JSON.stringify({
      params: {
        triggers: [{ id: "T", type: "contains", pattern: "HIT" }],
        isf_dir: "/tmp/isf",
      },
    }),
    "utf8"
  );
  const overlay = loadOverlayFile(file);
  const defs = [
    { name: "triggers", type: "json", path: "serial_triggers.items" },
    { name: "isf_dir", type: "path", path: "paths.isf_dir" },
  ];
  const args = mergeArgs(defs, overlay);
  const cfg = applyParamPaths(
    {
      gui: { api: "http://127.0.0.1:7878" },
      paths: { isf_dir: "old" },
      serial_triggers: { items: [] },
    },
    defs,
    args
  );
  assert.equal(cfg.paths.isf_dir, "/tmp/isf");
  assert.equal(cfg.serial_triggers.items[0].id, "T");
  fs.rmSync(dir, { recursive: true, force: true });
});

test("plugin.json group/advanced/hidden are accepted", () => {
  const check = validatePluginManifest({
    id: "demo",
    name: "Demo",
    report_name: "DemoReport",
    type: "custom",
    params: [
      { name: "a", type: "string", group: "paths", group_zh: "路径" },
      { name: "b", type: "json", advanced: true },
      { name: "preflight_only", type: "boolean", hidden: true },
    ],
  });
  assert.equal(check.ok, true, check.errors?.join("; "));
});

test("report_name is required on plugin.json", () => {
  const missing = validatePluginManifest({
    id: "demo",
    name: "Demo",
    type: "custom",
  });
  assert.equal(missing.ok, false);
  assert.ok(missing.errors.some((e) => e.includes("report_name")));
  assert.equal(
    resolveReportName({ id: "wiparse-dap", report_name: "WiparseDap" }, {}),
    "WiparseDap"
  );
  assert.equal(
    resolveReportName({ id: "wiparse-dap", report_name: "WiparseDap" }, { file_prefix: "MyLot" }),
    "MyLot"
  );
});

test("resolveProjectLayout run vs preflight", () => {
  const run = resolveProjectLayout({
    dataRoot: "/data",
    project: "default",
    testId: "scope-serial-monitor",
    stamp: "20260915_120000",
    lifecycle: "run",
  });
  assert.equal(run.stamp, "20260915_120000");
  assert.match(run.artifacts_dir.replaceAll("\\", "/"), /\/projects\/default\/tests\/scope-serial-monitor\/runs\/20260915_120000\/artifacts$/);

  const pf = resolveProjectLayout({
    dataRoot: "/data",
    project: "default",
    testId: "scope-serial-monitor",
    stamp: "20260915_120000",
    lifecycle: "preflight",
  });
  assert.equal(pf.stamp, "");
  assert.equal(pf.run_dir, "");
  assert.match(pf.test_dir.replaceAll("\\", "/"), /\/projects\/default\/tests\/scope-serial-monitor$/);
});

test("expandTemplates injects project layout and stamp on run", () => {
  const layout = resolveProjectLayout({
    dataRoot: "/data",
    project: "default",
    testId: "scope-serial-monitor",
    stamp: "20260915_120000",
    lifecycle: "run",
  });
  const raw = {
    station: { product: "P" },
    paths: {
      isf_dir: "{artifacts_dir}/waves",
      lock_file: "{test_dir}/run.lock",
      status_file: "{test_dir}/status.json",
    },
  };
  injectDefaultPathTemplates(raw, { hasRunDir: true });
  const cfg = expandTemplates(raw, { dataRoot: "/data", pluginDir: "/plug", layout });
  assert.match(cfg.paths.isf_dir.replaceAll("\\", "/"), /\/artifacts\/waves$/);
  assert.match(cfg.paths.lock_file.replaceAll("\\", "/"), /\/tests\/scope-serial-monitor\/run\.lock$/);
  assert.doesNotMatch(cfg.paths.lock_file, /plugin_dir|instrument_data/);
});

test("normalizeArtifacts drops paths outside run_dir and skip keys", () => {
  const runDir = path.join(os.tmpdir(), "wiparse-run-jail");
  fs.mkdirSync(runDir, { recursive: true });
  const inside = path.join(runDir, "artifacts", "waves", "a.isf");
  fs.mkdirSync(path.dirname(inside), { recursive: true });
  fs.writeFileSync(inside, "x");
  const out = normalizeArtifacts(
    {
      waves: [inside],
      stop_file: path.join(runDir, "..", "run.stop"),
      other: [path.join(os.tmpdir(), "outside.bin")],
    },
    { runDir, outputs: [{ id: "waves", view: "wave" }] }
  );
  assert.equal(out.items.length, 1);
  assert.equal(out.items[0].output, "waves");
  assert.equal(out.items[0].view, "wave");
  fs.rmSync(runDir, { recursive: true, force: true });
});

test("writeRunJson lands under runs/stamp", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "wiparse-layout-"));
  const stamp = makeStamp();
  const layout = resolveProjectLayout({
    dataRoot: root,
    project: "default",
    testId: "scope-serial-monitor",
    stamp,
    lifecycle: "run",
  });
  fs.mkdirSync(layout.artifacts_dir, { recursive: true });
  const dest = writeRunJson(layout, { id: "scope-serial-monitor", outputs: [] }, {
    ok: true,
    lifecycle: "run",
    session: stamp,
    artifacts: { items: [] },
  });
  assert.equal(path.basename(path.dirname(dest)), stamp);
  assert.equal(path.basename(dest), "run.json");
  const doc = JSON.parse(fs.readFileSync(dest, "utf8"));
  assert.equal(doc.schema, "wiparse.run/v1");
  assert.equal(doc.plugin, "scope-serial-monitor");
  fs.rmSync(root, { recursive: true, force: true });
});

test("ensureTestInProject appends only the running plugin", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "wiparse-proj-"));
  const layout = resolveProjectLayout({
    dataRoot: root,
    project: "lot1",
    testId: "example-smoke",
    stamp: makeStamp(),
    lifecycle: "run",
  });
  ensureTestInProject(layout, "example-smoke");
  ensureTestInProject(layout, "example-smoke");
  const doc = JSON.parse(fs.readFileSync(path.join(root, "projects", "lot1", "project.json"), "utf8"));
  assert.equal(doc.tests.length, 1);
  assert.equal(doc.tests[0].id, "example-smoke");
  fs.rmSync(root, { recursive: true, force: true });
});

test("ensureTestInProject uses report-name test_id with plugin_id", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "wiparse-proj-"));
  const layout = resolveProjectLayout({
    dataRoot: root,
    project: "lot1",
    testId: "ScopeSerial",
    pluginId: "example-smoke",
    stamp: makeStamp(),
    lifecycle: "run",
  });
  ensureTestInProject(layout, "example-smoke", { name: "ScopeSerial" });
  const doc = JSON.parse(fs.readFileSync(path.join(root, "projects", "lot1", "project.json"), "utf8"));
  assert.equal(doc.tests.length, 1);
  assert.equal(doc.tests[0].id, "ScopeSerial");
  assert.equal(doc.tests[0].plugin_id, "example-smoke");
  fs.rmSync(root, { recursive: true, force: true });
});

test("loadStationConfig derives test_id from file_prefix when layout is omitted", () => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), "wiparse-layout-"));
  const pluginDir = fs.mkdtempSync(path.join(os.tmpdir(), "wiparse-plugin-"));
  fs.writeFileSync(
    path.join(pluginDir, "station.json"),
    JSON.stringify({
      gui: { api: "http://127.0.0.1:7878" },
      paths: { file_prefix: "ScopeSerial" },
    })
  );
  const { layout, config } = loadStationConfig({
    plugin: { id: "scope-serial-monitor", dir: pluginDir, params: [] },
    pluginDir,
    args: {},
    dataRoot: root,
    project: "default",
    lifecycle: "preflight",
  });
  assert.equal(layout.test_id, "ScopeSerial");
  assert.equal(layout.plugin_id, "scope-serial-monitor");
  assert.ok(String(config.paths.test_dir).endsWith(`${path.sep}ScopeSerial`));
  assert.equal(sanitizeTestId("mpp25w test", "scope-serial-monitor"), "mpp25w_test");
  fs.rmSync(root, { recursive: true, force: true });
  fs.rmSync(pluginDir, { recursive: true, force: true });
});
