import test from 'node:test';
import assert from 'node:assert/strict';
import {HubError} from '../lib/hub/common.mjs';
import {retryWorkspaceLock} from '../lib/hub/retry.mjs';
const busy=()=>new HubError('CONFIG.BUSY','live owner',{lock_contention:true});

test('repository acquisition contention waits and enters the action exactly once',async()=>{
 let time=0,calls=0,entered=0,notices=0;
 const result=await retryWorkspaceLock(()=>{calls++;if(calls<3)throw busy();entered++;return 'bound';},
  {now:()=>time,sleep:async ms=>{time+=ms;},onWait:()=>notices++});
 assert.equal(result,'bound');assert.equal(calls,3);assert.equal(entered,1);assert.equal(notices,1);
});
test('repository waiting stops before entering an operation',async()=>{
 let cancelled=false,calls=0;
 await assert.rejects(retryWorkspaceLock(()=>{calls++;throw busy();},{stopped:()=>cancelled,sleep:async()=>{cancelled=true;}}),{code:'RUN.ABORTED'});
 assert.equal(calls,1);
});
test('repository waiting has a fixed deadline',async()=>{
 let time=0,calls=0;
 await assert.rejects(retryWorkspaceLock(()=>{calls++;throw busy();},{now:()=>time,sleep:async ms=>{time+=ms;},timeoutMs:20,intervalMs:5}),{code:'CONFIG.BUSY'});
 assert.equal(time,20);assert.equal(calls,4);
});
test('recovery, stale bindings, I/O failures and business BUSY are never replayed',async()=>{
 for(const error of [new HubError('CONFIG.RECOVERY_REQUIRED'),new HubError('PREFLIGHT.STALE'),new HubError('IO.TIMEOUT'),new HubError('CONFIG.BUSY')]) {
  let calls=0;await assert.rejects(retryWorkspaceLock(()=>{calls++;throw error;},{sleep:async()=>assert.fail('Unexpected retry')}),e=>e===error);assert.equal(calls,1);
 }
});
