import test from 'node:test';
import assert from 'node:assert/strict';
import {RequestEpoch} from '../src/requestEpoch.ts';

test('out-of-order report indexing cannot replace a newer read or its loading state', async () => {
  const gate=new RequestEpoch();gate.setScope('project-a:visible');
  let finishOld,finishNew;
  const old=new Promise(resolve=>{finishOld=resolve;}),latest=new Promise(resolve=>{finishNew=resolve;});
  const displayed=[];const a=gate.begin();const first=old.then(value=>{if(gate.isCurrent(a))displayed.push(value);});
  const b=gate.begin();const second=latest.then(value=>{if(gate.isCurrent(b))displayed.push(value);});
  finishNew('current');await second;finishOld('stale');await first;
  assert.deepEqual(displayed,['current']);assert.equal(gate.isCurrent(a),false);
});
test('project changes and hiding invalidate results even when returning to the same project', () => {
  const gate=new RequestEpoch();gate.setScope('project-a:visible');const old=gate.begin();
  gate.setScope('project-b:visible');gate.setScope('project-a:visible');assert.equal(gate.isCurrent(old),false);
  const visible=gate.begin();gate.setScope('project-a:hidden');gate.setScope('project-a:visible');assert.equal(gate.isCurrent(visible),false);
  const active=gate.begin();gate.setScope('project-a:visible');assert.equal(gate.isCurrent(active),true);
  gate.invalidate();assert.equal(gate.isCurrent(active),false);
});

test('mutation completion leases expire on hide, disconnect and project change without cancelling the host action', () => {
  const context=new RequestEpoch();context.setScope('project-a:connected:visible');
  const action=context.capture();assert.equal(context.isCurrent(action),true);
  context.setScope('project-a:connected:hidden');assert.equal(context.isCurrent(action),false);
  context.setScope('project-a:connected:visible');assert.equal(context.isCurrent(action),false);
  const another=context.capture();context.setScope('project-a:disconnected:visible');assert.equal(context.isCurrent(another),false);
});
