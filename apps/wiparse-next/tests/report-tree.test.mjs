import test from 'node:test';
import assert from 'node:assert/strict';
import {artifactTitle, canRemoveProjectTest, reportProjectMatches, latestMarkdown, latestRunReport} from '../src/reportTree.ts';

test('latest markdown ignores newer logs and keeps the last equal mtime after path order', () => {
  const chosen = latestMarkdown([
    {name: 'b.md', path: 'b.md', mtime: 10},
    {name: 'a.md', path: 'a.md', mtime: 10},
    {name: 'notes.log', path: 'notes.log', mtime: 99},
    {name: 'copy.md', path: 'a.md', mtime: 1},
  ]);
  assert.equal(chosen?.path, 'b.md');
  assert.equal(latestMarkdown([{name: 'only.log', path: 'only.log', mtime: 5}]), null);
});

test('a test report follows run order when modification times match', () => {
  const older = {name: 'old.md', path: 'old.md', mtime: 10};
  const newer = {name: 'new.md', path: 'new.md', mtime: 10};
  assert.equal(latestRunReport([{latest: newer}, {latest: older}])?.path, 'old.md');
  assert.equal(latestRunReport([{latest: null}, {latest: newer}])?.path, 'new.md');
  assert.equal(canRemoveProjectTest('lot1', 'example-smoke'), true);
  assert.equal(canRemoveProjectTest('', 'example-smoke'), false);
  assert.equal(canRemoveProjectTest('lot1', '../other'), false);
  assert.equal(canRemoveProjectTest('lot1', 'a'.repeat(129)), false);
  assert.equal(artifactTitle('summary.md', '', ''), '总报告');
  assert.equal(artifactTitle('lot_summary_20260915_143738.md', 'lot', '20260915_143738'), '总报告 14:37:38');
  assert.equal(artifactTitle('lot_timeout_20260915_143738.isf', 'lot', '20260915_143738'), 'timeout 14:37:38');
});


test('report confirmation cannot transfer to a newly indexed project with the same test id', () => {
  assert.equal(reportProjectMatches('project-a','project-a','project-a'),true);
  assert.equal(reportProjectMatches('project-b','project-b','project-a'),false);
  assert.equal(reportProjectMatches('project-b','project-a','project-a'),false);
  assert.equal(reportProjectMatches('project-a','project-b','project-a'),false);
});
test('adding tests requires a current valid indexed project', () => {
  assert.equal(reportProjectMatches('project-a','project-a'),true);
  assert.equal(reportProjectMatches('',''),false);
  assert.equal(reportProjectMatches('../other','../other'),false);
  assert.equal(reportProjectMatches('project-b','project-a'),false);
});
