import test from 'node:test';
import assert from 'node:assert/strict';
import {inspectTerminalExport,findAlertRecoveryCandidates} from '../plugins/wireless-whitebox/mt5820-log.mjs';

test('terminal blocks expose observations without inferring timing, units or split fields',()=>{
  const data='[17:26:06.222]收←◆FW 1.1.d636 Compiler K\nStart CHG\n'+
    '[17:26:07.000]收←◆Ptx=6786 Ploss=-1192 FodCnt = 2 EPT_REASON:0\n'+
    '[17:26:08.000]收←◆Plos\n[17:26:08.010]收←◆s=500\n';
  const r=inspectTerminalExport(data);
  assert.equal(r.blocks,4);assert.equal(r.events.filter(e=>e.kind==='ploss_observed').length,1);
  const loss=r.events.find(e=>e.kind==='ploss_observed');
  assert.equal(loss.fields.raw_value,-1192);assert.equal(loss.fields.unit,'unknown');
  assert.equal(loss.time_source,'terminal_receive_block');assert.equal(data.slice(loss.offset,loss.offset+loss.raw.length),loss.raw);
  assert.equal(r.events.find(e=>e.kind==='ept_reason_observed').fields.meaning,'unconfirmed');
  assert.equal(r.complete,false);assert.equal(r.product_verdict,'INCONCLUSIVE');
});

test('FOD alert investigation retains evidence order and avoids manufacturing a verdict',()=>{
  const text='[17:27:47.726]收←◆FodCnt = 5\nFO Alert!\n'+
   '[17:27:47.909]收←◆Reset Vbrg: 9000 Vbrg_act: 7763\n'+
   '[17:27:48.026]收←◆EPT_REASON:2000\n'+
   '[17:27:58.100]收←◆Start CHG\n';
  const r=inspectTerminalExport(text), [candidate]=findAlertRecoveryCandidates(r.events);
  assert.equal(candidate.prior_count.fields.raw_value,5);
  assert.equal(candidate.reason.fields.raw_value,2000);
  assert.equal(candidate.next_charge.receive_time,'17:27:58.100');
  assert.equal(candidate.verdict,'INCONCLUSIVE');
  assert.equal(findAlertRecoveryCandidates(inspectTerminalExport('[00:00:00.000]收←◆Start CHG').events).length,0);
});
