import test from 'node:test';
import assert from 'node:assert/strict';
import {withConstantCurrentLoad} from '../plugins/wireless-whitebox/load-sequence.mjs';

test('bounded CC load applies off, mode, level, on, measure, off in order',async()=>{
  const steps=[];
  const result=await withConstantCurrentLoad({current_a:0.5,maximum_a:1,command:async c=>steps.push(c),markUncertain:async()=>assert.fail('unexpected recovery')},async()=>{steps.push('measure');return 5.1;});
  assert.deepEqual(steps,[{LoadInput:false},{LoadMode:'CC'},{LoadLevel:{mode:'CC',value:0.5}},{LoadInput:true},'measure',{LoadInput:false}]);
  assert.equal(result.value,5.1);assert.equal(result.load.physical_off_verified,false);
});
test('limit error sends no command, action error still turns load off',async()=>{
  let calls=[];const opts={current_a:2,maximum_a:1,command:async c=>calls.push(c),markUncertain:async()=>assert.fail()};
  await assert.rejects(withConstantCurrentLoad(opts,async()=>{}),/LIMIT_INVALID/);assert.equal(calls.length,0);
  opts.current_a=0.5;await assert.rejects(withConstantCurrentLoad(opts,async()=>{throw new Error('MEASURE.FAILED');}),/MEASURE.FAILED/);
  assert.deepEqual(calls.at(-1),{LoadInput:false});
});
test('unknown command state and failed shutdown retain recovery without retry',async()=>{
  let calls=[],recovery=[];
  const opts={current_a:0.5,maximum_a:1,command:async c=>{calls.push(c);if(c.LoadInput===true)throw new Error('timeout');},markUncertain:async r=>recovery.push(r)};
  await assert.rejects(withConstantCurrentLoad(opts,async()=>{}),/timeout/);assert.deepEqual(recovery,['LOAD_STATE_UNKNOWN']);assert.equal(calls.length,4);
  calls=[];recovery=[];opts.command=async c=>{calls.push(c);if(c.LoadInput===false&&calls.length===5){const e=new Error('off failed');e.quiescent=true;throw e;}};
  await assert.rejects(withConstantCurrentLoad(opts,async()=>42),/off failed/);assert.deepEqual(recovery,['LOAD_SHUTDOWN_FAILED']);assert.equal(calls.length,5);
});
test('stop after enabling the load still sends the shutdown command',async()=>{
  let stopped=false;const calls=[];
  const opts={current_a:0.1,maximum_a:1,stopped:()=>stopped,command:async c=>{calls.push(c);if(c.LoadInput===true)stopped=true;},markUncertain:async()=>assert.fail()};
  await assert.rejects(withConstantCurrentLoad(opts,async()=>assert.fail()),/RUN.ABORTED/);
  assert.deepEqual(calls.at(-1),{LoadInput:false});
});
test('an unresolved instrument operation is never overlapped by shutdown',async()=>{
  let uncertain=false;const calls=[];
  const opts={current_a:0.1,maximum_a:1,isUncertain:()=>uncertain,command:async c=>calls.push(c),markUncertain:async()=>assert.fail()};
  await assert.rejects(withConstantCurrentLoad(opts,async()=>{uncertain=true;throw new Error('IO.STATE_UNKNOWN');}),/STATE_UNKNOWN/);
  assert.equal(calls.length,4);assert.deepEqual(calls.at(-1),{LoadInput:true});
});
