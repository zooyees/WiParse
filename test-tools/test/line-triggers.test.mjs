import test from "node:test";
import assert from "node:assert/strict";
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
