import test from 'node:test';
import assert from 'node:assert/strict';
import {autoLayout, cursorX, displayExtent, displayY, dragOffset, dragScale, fitY, formatAxisTick, formatEngShort, formatPlain, frameAt, nativeY, nearestHandle, nearestSeries, niceAxisTicks, perDivision, sampleAt, thresholdY, wheelAdjust, wheelPixels, zoomY} from '../src/dataDisplay.ts';

test('display mapping keeps the old full-scale spans and kind bands', () => {
  assert.equal(displayY(10_000, 1, 0, 'voltage'), 1);
  assert.equal(displayY(5_000, 1, 0, 'current'), 1);
  assert.equal(displayY(50, 1, 0, 'temperature'), 1);
  assert.equal(displayY(10_000, 2, 1.2, 'voltage'), 3.2);
  const layout = autoLayout([
    {kind: 'temperature', active: true},
    {kind: 'voltage', active: true},
    {kind: 'voltage', active: false},
    {kind: 'current', active: true},
  ]);
  assert.deepEqual(layout.map(item => item.offset), [4.8, 0, 0, 2.4]);
  assert.equal(layout[1].scale, 1);
});

test('the shared axis fits mapped extrema and zooms around the center', () => {
  const extent = displayExtent([
    {minimum: 0, maximum: 10_000, kind: 'voltage', scale: 1, offset: 0},
    {minimum: null, maximum: null, kind: 'current', scale: 1, offset: 1.2},
  ]);
  assert.deepEqual(extent, {min: 0, max: 1});
  assert.deepEqual(fitY(extent), {min: -0.08, max: 1.08});
  assert.deepEqual(zoomY({min: 0, max: 2}, 0.5), {min: 0.5, max: 1.5});
  assert.equal(zoomY({min: 0, max: 0}, 0.5), null);
  assert.equal(thresholdY(5_000, [
    {kind: 'voltage', scale: 1, offset: 0, active: true},
    {kind: 'current', scale: 1, offset: 1.2, active: true},
  ], 'current'), 2.2);
  assert.equal(thresholdY(1, [{kind: 'voltage', scale: 1, offset: 0, active: true}], 'temperature'), null);
});

test('dragging a handle follows the old offset and scale formulas', () => {
  assert.equal(dragOffset(1.2, 20, 200, 2), 1);
  assert.equal(dragOffset(1.2, 0, 0, 2), 1.2);
  assert.ok(Math.abs(dragScale(1, -10) - Math.exp(0.08)) < 1e-12);
  assert.equal(dragScale(1e12, -1000), 1e12);
  assert.equal(wheelPixels(1, 1), 120);
  assert.equal(wheelPixels(40, 0), 40);
  assert.equal(wheelAdjust({offset: 1, scale: 1}, 120, 2, true)?.offset, 1 - 0.16);
  assert.equal(wheelAdjust({offset: 1, scale: 1}, 0, 2, true), null);
  assert.ok(Math.abs((wheelAdjust({offset: 0, scale: 1}, 100, 2, false)?.scale ?? 0) - Math.exp(0.2)) < 1e-12);
  assert.equal(wheelAdjust({offset: 0, scale: 1e12}, 1e9, 2, false)?.scale, 1e12);
  assert.equal(nearestHandle([{x: 10, y: 10, active: true}, {x: 40, y: 10, active: true}], 12, 11), 0);
  assert.equal(nearestHandle([{x: 10, y: 10, active: false}], 10, 10), null);
  assert.ok(Math.abs(nativeY(displayY(5_000, 2, 1.2, 'voltage'), 2, 1.2, 'voltage') - 5_000) < 1e-6);
  assert.equal(perDivision(0, 10_000), 1_250);
  assert.deepEqual(niceAxisTicks(0, 100, 5), [0, 50, 100]);
  assert.deepEqual(niceAxisTicks(0, 0, 5), [0]);
  assert.equal(formatEngShort(1_000), '1.000e3');
  assert.equal(formatEngShort(12), '12');
  assert.equal(formatAxisTick(10_000, 'mV', true), '1.00e4mV');
  assert.equal(formatAxisTick(12.34, 'mV', false), '12.3');
  assert.equal(formatPlain(1500), '1500');
  assert.equal(formatPlain(1500.26), '1500.3');
  assert.equal(formatPlain(1.2), '1.200');
  assert.deepEqual(sampleAt([{frame: 0, low: 0, high: 0}, {frame: 2, low: 10, high: 10}], 1), {kind: 'value', value: 5});
  assert.deepEqual(sampleAt([{frame: 0, low: 0, high: 0}, {frame: 2, low: 10, high: 10}], 9), {kind: 'value', value: 10});
  assert.deepEqual(sampleAt([{frame: 0, low: 1, high: 4}, {frame: 10, low: 0, high: 0}], 2), {kind: 'range', low: 1, high: 4});
  const plotted = [
    {buckets: [{frame: 0, low: 0, high: 0}], scale: 1, offset: 0, kind: 'voltage', active: true},
    {buckets: [{frame: 0, low: 5_000, high: 5_000}], scale: 1, offset: 0, kind: 'current', active: true},
  ];
  assert.equal(nearestSeries(plotted, 0, 0.1), 0);
  assert.equal(nearestSeries(plotted, 0, 0.9), 1);
  assert.equal(nearestSeries([{buckets: [{frame: 0, low: 0, high: 10_000}], scale: 1, offset: 0, kind: 'voltage', active: true}], 0, 0.4), 0);
  assert.equal(nearestSeries([], 0, 0), null);
  assert.equal(frameAt(60, 10, 100, 0, 11), 5);
  assert.equal(cursorX(5, 10, 100, 0, 11), 60);
  assert.equal(frameAt(9, 10, 100, 0, 11), null);
  assert.equal(cursorX(5, 10, 0, 0, 11), null);
});
