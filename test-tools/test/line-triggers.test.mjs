import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import { validatePluginManifest } from "../lib/plugin-contract.mjs";
import {
  classifyLine,
  compileTriggers,
  createLineClassifier,
  parseJsonValue,
  rawTriggerItems,
} from "../lib/line-triggers.mjs";

const recipe = [
  {
    id: "ASK2",
    type: "regex",
    pattern: "ASK\\s+0?2\\b",
    unless: ["ASK\\s+0?2\\s+[cC]\\b", "ASK\\s+0?2\\s+[bB]\\b", "Idx\\[0?2\\]"],
  },
  {
    id: "timeout",
    type: "regex",
    flags: "i",
    pattern: "\\btimeout\\b",
  },
  {
    id: "ASK71",
    enabled: false,
    type: "regex",
    pattern: "ASK\\s+0?71\\b",
  },
];

test("compileTriggers skips disabled and honors unless", () => {
  const compiled = compileTriggers({ items: recipe });
  assert.deepEqual(
    compiled.map((t) => t.id),
    ["ASK2", "timeout"]
  );
  const ask2 = compiled.find((t) => t.id === "ASK2");
  assert.equal(ask2.match("ASK 2 foo"), true);
  assert.equal(ask2.match("ASK 2 C"), false);
  assert.equal(ask2.match("ASK 2 b"), false);
  assert.equal(ask2.match("Idx[2]"), false);
  const to = compiled.find((t) => t.id === "timeout");
  assert.equal(to.match("device TIMEOUT now"), true);
});

test("contains match and Hub JSON string overlay", () => {
  const fromHub = JSON.stringify([
    { id: "nack", type: "contains", pattern: "NACK" },
  ]);
  const items = rawTriggerItems(parseJsonValue(fromHub, { label: "triggers" }));
  const compiled = compileTriggers(items);
  assert.equal(compiled[0].match("got nack here"), true);
  assert.equal(compiled[0].match("got NACK here"), true);
});

test("rising-edge fires once per inactive→active", () => {
  const c = createLineClassifier({ items: [{ id: "T", type: "contains", pattern: "HIT" }] });
  assert.equal(c.classify("noise"), null);
  assert.equal(c.classify("HIT 1"), "T");
  assert.equal(c.classify("HIT 2"), null);
  assert.equal(c.classify("idle"), null);
  assert.equal(c.classify("HIT 3"), "T");
});

test("level match (rising_edge false) fires every line", () => {
  const compiled = compileTriggers({
    items: [{ id: "T", type: "contains", pattern: "HIT" }],
  });
  assert.equal(classifyLine(compiled, "HIT", { risingEdge: false }), "T");
  assert.equal(classifyLine(compiled, "HIT", { risingEdge: false }), "T");
});

test("first_match: earlier rule wins on the same line", () => {
  const c = createLineClassifier({
    items: [
      { id: "A", type: "contains", pattern: "ASK" },
      { id: "B", type: "contains", pattern: "ASK 2" },
    ],
  });
  assert.equal(c.classify("ASK 2"), "A");
});

test("global and sticky regexes do not leak lastIndex across lines", () => {
  for (const flags of ["g", "y"]) {
    const c = createLineClassifier({
      items: [{ id: "T", type: "regex", pattern: "ASK", flags }],
    });
    assert.equal(c.classify("ASK"), "T");
    assert.equal(c.classify("ASK"), null);
    assert.equal(c.classify("ASK"), null);
    assert.equal(c.classify("idle"), null);
    assert.equal(c.classify("ASK"), "T");
  }
});

test("global exclude regex is reset before every line", () => {
  const [trigger] = compileTriggers({
    items: [{ id: "T", pattern: "ASK", exclude: "BAD", exclude_flags: "g" }],
  });
  assert.equal(trigger.match("ASK BAD"), false);
  assert.equal(trigger.match("ASK BAD"), false);
});

test("visual string rules preserve literal punctuation, case and exclusions", () => {
  const [rule] = compileTriggers([{ id: "literal", type: "contains", pattern: "ERROR[2].*",
    case_sensitive: true, unless: [{ type: "contains", pattern: "ignored", case_sensitive: false }] }]);
  assert.equal(rule.match("ERROR[2].* detail"), true);
  assert.equal(rule.match("error[2].* detail"), false);
  assert.equal(rule.match("ERROR22 detail"), false);
  assert.equal(rule.match("ERROR[2].* IGNORED"), false);
});

test("duplicate enabled IDs including sanitized collisions are rejected", () => {
  for (const ids of [["T", "T"], ["a b", "a?b"]]) {
    assert.throws(() => compileTriggers(ids.map(id => ({ id, type: "contains", pattern: "X" }))), /id 重复/);
  }
  assert.equal(compileTriggers([{ id: "T", pattern: "X" }, { id: "T", enabled: false }]).length, 1);
});

test("visual editor manifest retains JSON overlays and the factory recipe", () => {
  const root = new URL("../plugins/scope-serial-monitor/", import.meta.url);
  const plugin = JSON.parse(fs.readFileSync(new URL("plugin.json", root)));
  assert.deepEqual(validatePluginManifest(plugin), { ok: true, errors: [] });
  const station = JSON.parse(fs.readFileSync(new URL("station.json", root)));
  const param = plugin.params.find(p => p.name === "triggers");
  assert.equal(param.type, "json");
  assert.equal(param.widget, "line_triggers");
  assert.equal(param.path, "serial_triggers.items");
  const rules = compileTriggers(station.serial_triggers);
  assert.equal(rules.find(t => t.id === "ASK2").match("ASK 2 error"), true);
  assert.equal(rules.find(t => t.id === "ASK2").match("ASK 2 C"), false);
  assert.equal(rules.find(t => t.id === "timeout").match("TIMEOUT"), true);
  assert(!rules.some(t => t.id === "ASK71"));
});
