import test from 'node:test';
import assert from 'node:assert/strict';
import {bandpassGeometry, horizontalResistor, verticalResistor} from '../src/bandpassGeometry.ts';

test('bandpass schematic nodes stay inside the drawing and run left to right', () => {
  const rect = {x: 10, y: 20, width: 300, height: 160};
  const geometry = bandpassGeometry(rect);
  const nodes = [geometry.vin, geometry.c1Left, geometry.c1Right, geometry.highPass, geometry.bufferIn, geometry.bufferOut, geometry.r2Left, geometry.r2Right, geometry.vout];
  assert.equal(geometry.vin.x, 22);
  assert.equal(geometry.vin.y, 80.8);
  assert.equal(geometry.groundY, 157.6);
  assert.ok(nodes.every(point => Number.isFinite(point.x) && Number.isFinite(point.y)));
  assert.ok(nodes.every((point, index) => index === 0 || nodes[index - 1].x < point.x));
  assert.ok(geometry.groundY > geometry.vin.y);
  assert.ok(nodes.every(point => point.x >= rect.x && point.x <= rect.x + rect.width && point.y >= rect.y && point.y <= rect.y + rect.height));
  assert.equal(horizontalResistor({x: 0, y: 10}, {x: 80, y: 10})[2].y, 5);
  assert.equal(verticalResistor(4, 0, 80)[1].x, 9);
});
