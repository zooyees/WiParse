import test from 'node:test';
import assert from 'node:assert/strict';
import {openTarget} from '../src/reportOpen.ts';

test('report artifacts follow the old extension and group view', () => {
  assert.equal(openTarget('report.markdown', 'external', 'log'), 'report');
  assert.equal(openTarget('scope.ISF', 'log', 'log'), 'wave');
  assert.equal(openTarget('capture.wfm', 'external', 'external'), 'wave');
  assert.equal(openTarget('table.csv', 'log', 'log'), 'wave');
  assert.equal(openTarget('shot.PNG', 'wave', 'wave'), 'external');
  assert.equal(openTarget('page.html', 'report', 'report'), 'external');
  assert.equal(openTarget('run.log', 'wave', 'wave'), 'text');
  assert.equal(openTarget('frames.jsonl', 'external', 'external'), 'text');
  assert.equal(openTarget('notes.txt', 'report', 'wave'), 'wave');
  assert.equal(openTarget('notes.txt', 'wave', 'log'), 'text');
  assert.equal(openTarget('notes.txt', 'log', 'report'), 'report');
  assert.equal(openTarget('notes.txt', 'log', 'external'), 'external');
  assert.equal(openTarget('firmware.bin', 'wave', 'log'), 'wave');
  assert.equal(openTarget('firmware.bin', 'report', 'wave'), 'report');
  assert.equal(openTarget('readme', 'log', 'wave'), 'text');
});
