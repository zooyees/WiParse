import test from 'node:test';
import assert from 'node:assert/strict';
import {crcParameters} from '../src/crcPreset.ts';

const custom = {width: 16, poly: '0x1021', init: '0xFFFF', xorout: '0x0000', refin: false, refout: false};

test('crc parameter table matches the old presets and hides an invalid custom polynomial', () => {
  assert.deepEqual(crcParameters('CRC-32/ISO-HDLC', custom), {width: 32, poly: '0x04C11DB7', init: '0xFFFFFFFF', refin: true, refout: true, xorout: '0xFFFFFFFF'});
  assert.deepEqual(crcParameters('CRC-8', custom), {width: 8, poly: '0x07', init: '0x00', refin: false, refout: false, xorout: '0x00'});
  assert.equal(crcParameters('Custom', custom)?.poly, '0x1021');
  assert.equal(crcParameters('Custom', {...custom, poly: '0'}), null);
  assert.equal(crcParameters('Custom', {...custom, poly: '0x1FFFF'}), null);
});
