import test from 'node:test';
import assert from 'node:assert/strict';
import {settlingNote, settlingPercent} from '../src/rcSettling.ts';

test('rc settling matches the old one, three and five tau percentages', () => {
  for (const [multiple, complete, remaining] of [[1, 63.212, 36.788], [3, 95.021, 4.979], [5, 99.326, 0.674]]) {
    const value = settlingPercent(multiple);
    assert.ok(Math.abs(value.complete - complete) < 0.001);
    assert.ok(Math.abs(value.remaining - remaining) < 0.001);
  }
  assert.equal(settlingNote(), '1τ: 完成63.212%/误差36.788% · 3τ: 95.021%/4.979% · 5τ: 99.326%/0.674%');
});
