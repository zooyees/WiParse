import test from 'node:test';
import assert from 'node:assert/strict';
import {navigationRequest} from '../src/navigation.ts';
test('platform navigation uses only current explicit requests and known pages',()=>{
  assert.deepEqual(navigationRequest({owner:42,revision:2,state:'requested',tab:'waveform'}),{owner:42,revision:2,page:'wave'});
  for(const value of [{owner:42,revision:2,state:'applied',tab:'serial'},{owner:42,revision:0,state:'requested',tab:'serial'},{owner:42,revision:2,state:'requested',tab:'javascript:run'},{owner:0,revision:2,state:'requested',tab:'serial'}])assert.equal(navigationRequest(value),null);
});
