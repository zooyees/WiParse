import test from 'node:test';
import assert from 'node:assert/strict';
import {liveStartRow,liveRulesKey} from '../src/dataLive.ts';

test('serial live starts at the recent tail and skips rows the log has already dropped', () => {
  assert.equal(liveStartRow(null, 100, 0), 0);
  assert.equal(liveStartRow(null, 5000, 0), 3000);
  assert.equal(liveStartRow(0, 5000, 4000), 4000);
  assert.equal(liveStartRow(4500, 5000, 4200), 4500);
  assert.equal(liveStartRow(100, 5000, 200), 200);
  assert.equal(liveStartRow(null, Number.NaN, 0), 0);
});

test('live analysis identity normalizes whitespace but preserves exact header/filter semantics',()=>{
  assert.equal(liveRulesKey(' FRAME ',[' TEMP ','VIN']),liveRulesKey('FRAME',['TEMP','VIN']));
  assert.notEqual(liveRulesKey('FRAME',['TEMP','VIN']),liveRulesKey('FRAME',['VIN','TEMP']));
  assert.notEqual(liveRulesKey('FRAME',['TEMP']),liveRulesKey('FRAME',['temp']));
  assert.notEqual(liveRulesKey('A|B',['C']),liveRulesKey('A',['B|C']));
  assert.notEqual(liveRulesKey('FRAME',['TEMP']),liveRulesKey('OTHER',['TEMP']));
});
