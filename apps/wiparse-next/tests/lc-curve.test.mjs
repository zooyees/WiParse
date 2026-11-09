import test from 'node:test';
import assert from 'node:assert/strict';
import {resonancePoints} from '../src/lcCurve.ts';

test('resonance points stay in frequency order and drop invented samples', () => {
  assert.deepEqual(resonancePoints([[1, 0], [2, 0.5]]), [[1, 0], [2, 0.5]]);
  assert.deepEqual(resonancePoints([[2, 1], [1, 1]]), []);
  assert.deepEqual(resonancePoints([[1, -0.1], [2, 1]]), []);
  assert.deepEqual(resonancePoints('curve'), []);
});
