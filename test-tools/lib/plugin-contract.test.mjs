import test from "node:test";
import assert from "node:assert/strict";
import {
  pickInstrument,
  suggestedParamsFromInstrument,
  validatePluginManifest,
} from "./plugin-contract.mjs";
import fs from "node:fs";
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
  assert.equal(raw.params.some((p) => p.type === "device"), true);
});
