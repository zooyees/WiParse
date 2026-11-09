import test from 'node:test';
import assert from 'node:assert/strict';
import {formatI128, parseI128, radixSummary} from '../src/radixValue.ts';

test('radix conversion matches the old signed i128 cases', () => {
  assert.equal(formatI128(parseI128('0b1111', 2), 16), 'F');
  assert.equal(formatI128(parseI128('17', 8), 10), '15');
  assert.equal(formatI128(parseI128('255', 10), 16), 'FF');
  assert.equal(formatI128(parseI128('0xFF', 16), 2), '11111111');
  assert.equal(formatI128(parseI128('-0xFF', 16), 10), '-255');
  assert.equal(formatI128(parseI128('Z', 36), 10), '35');
  assert.equal(parseI128('2', 2), null);
  assert.equal(parseI128('1', 1), null);
  assert.equal(parseI128('170141183460469231731687303715884105728', 10), null);
  assert.equal(parseI128('-170141183460469231731687303715884105728', 10), -(1n << 127n));
  assert.equal(radixSummary('255', 10), 'BIN 11111111  ·  OCT 377  ·  DEC 255  ·  HEX FF');
  assert.equal(radixSummary('2', 2), null);
});
