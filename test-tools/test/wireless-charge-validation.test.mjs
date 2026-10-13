import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {loadBundle,preflightBundle,readDocument,hash} from '../plugins/wireless-charge-validation/lib/config.mjs';
import {aggregate,stateRule,communicationRule} from '../plugins/wireless-charge-validation/lib/rules.mjs';
import {execute} from '../plugins/wireless-charge-validation/lib/orchestrator.mjs';
import runPlugin,{preflight,stop} from '../plugins/wireless-charge-validation/index.mjs';
import {validatePluginManifest,normalizeResult,writeRunJson} from '../lib/plugin-contract.mjs';
import {AutomationClient} from '../lib/automation-client.mjs';
const root=path.resolve(path.dirname(fileURLToPath(import.meta.url)),'../plugins/wireless-charge-validation');
const bundle=(mode='replay',fixture='gold')=>loadBundle(root,{suite:'regression',mode,fixture});
const mockClient={async request(route,{method,params}) {
  assert.equal(route,'/v1/invoke');assert.equal(method,'check.evaluate');
  const {measurement:m,limits:l}=params;let v='PASS';
  if(!l.approved || m.value===null||m.sample_count===0||m.quality.issues.length||m.uncertainty===null) v='INCONCLUSIVE';
  else if(m.value+m.uncertainty<l.minimum||m.value-m.uncertainty>l.maximum)v='FAIL';
  else if(m.value-m.uncertainty<l.minimum||m.value+m.uncertainty>l.maximum)v='INCONCLUSIVE';
  return {schema_version:'wiparse.automation/1',metric_id:m.metric_id,limits_ref:l.limits_ref,limits_hash:l.limits_hash,product_verdict:v,reason_code:v==='PASS'?'CHECK.IN_RANGE':'CHECK.'+v,evidence_refs:m.evidence_refs};
}};
async function temporary(fn) {const dir=fs.mkdtempSync(path.join(os.tmpdir(),'wireless-plugin-test-'));try{return await fn(dir);}finally{fs.rmSync(dir,{recursive:true,force:true});}}
test('manifest accepted and SYS-01..10 have cases; forbidden limit/safety overlays rejected',()=>{
  assert.equal(validatePluginManifest(readDocument(root,'plugin.json').value).ok,true);
  const b=bundle();assert.equal(new Set(b.cases.flatMap(c=>c.sys)).size,10);
  for(const key of ['limits','approved','station','unsafe','minimum','iterations']) assert.throws(()=>loadBundle(root,{[key]:'x'}),/OVERLAY_FORBIDDEN/);
  assert.throws(()=>loadBundle(root,{product:'../escape'}),/INVALID_ID/);
  assert.throws(()=>loadBundle(root,{mode:'unknown'}),/INVALID_MODE/);
});
test('preflight and idle stop never allocate a run or stimulate',async()=>temporary(async(dir)=>{
  const ctx={pluginDir:root,plugin:readDocument(root,'plugin.json').value,args:{mode:'dry-run'},dataRoot:dir,project:'test'};
  assert.equal((await preflight(ctx)).ok,true);assert.deepEqual(fs.readdirSync(dir),[]);
  assert.equal((await stop(ctx)).ok,true);assert.deepEqual(fs.readdirSync(dir),[]);
}));
test('live blocks even when capabilities asserted; all dependency reasons visible',()=>{
  const b=bundle('live');const r=preflightBundle(b,{implemented:b.cases.flatMap(c=>c.required_capabilities),industrial_approved:true});
  assert.equal(r.ok,false);assert(r.blocking.includes('INTEGRATION.LIVE_BACKEND_NOT_VALIDATED'));
  assert(r.blocking.includes('LIMITS.NOT_PRODUCT_APPROVED'));assert(r.blocking.includes('SAFETY.NOT_VERIFIED'));
});
for(const [fixture,expected] of [['gold','PASS'],['bad','FAIL'],['missing','INCONCLUSIVE'],['quality','INCONCLUSIVE']]) {
  test(`${fixture} replay validates all SYS and never certifies a DUT`,async()=>temporary(async(dir)=>{
    const run=await execute(bundle('replay',fixture),{root,dir,client:mockClient});
    assert.equal(run.execution_status,'completed');assert.equal(run.validation_verdict,expected);assert.equal(run.product_verdict,'INCONCLUSIVE');
    const coverage=JSON.parse(fs.readFileSync(path.join(dir,'coverage.json')));assert.equal(coverage.length,10);
    for(const row of coverage){assert.equal(row.missing_case_ids.length,0);assert.equal(row.verdict,expected);}
    const manifest=JSON.parse(fs.readFileSync(path.join(dir,'manifest.json')));
    for(const file of manifest.files)assert.equal(hash(fs.readFileSync(path.join(dir,file.path))),file.sha256);
    assert.equal(run.cases.length,bundle().cases.length);
  }));
}
test('dry-run has no measurements/PASS; no service calls',async()=>temporary(async(dir)=>{
  const run=await execute(bundle('dry-run'),{root,dir,client:{request(){throw new Error('unexpected');}}});
  assert.equal(run.validation_verdict,'INCONCLUSIVE');assert(run.cases.every(c=>c.product_verdict==='INCONCLUSIVE'));
}));
test('cancel after confirmed failure preserves first failure and remaining coverage',async()=>temporary(async(dir)=>{
  const controller=new AbortController();let count=0;
  const client={async request(...args){const r=await mockClient.request(...args);if(++count===1)controller.abort();return r;}};
  const run=await execute(bundle('replay','bad'),{root,dir,client,signal:controller.signal});
  assert.equal(run.execution_status,'cancelled');assert.equal(run.validation_verdict,'FAIL');assert.equal(run.cases[0].product_verdict,'FAIL');
  assert(run.errors.every(e=>e.first_error_id===run.first_error_id));assert.equal(run.cases.length,bundle().cases.length);
}));
test('unknown service state after a FAIL remains interrupted and never retries',async()=>temporary(async(dir)=>{
  let count=0;const client={async request(...args){if(++count===2)throw new Error('JOB.UNKNOWN');return mockClient.request(...args);}};
  const run=await execute(bundle('replay','bad'),{root,dir,client});
  assert.equal(count,2);assert.equal(run.execution_status,'interrupted');assert.equal(run.validation_verdict,'FAIL');
}));
test('null/empty/uncertain/state mismatch/window/channel mismatch cannot pass',async()=>{
  for(const change of [o=>o.measurement.value=null,o=>o.measurement.sample_count=0,o=>o.measurement.uncertainty=null,o=>o.state='IDLE',o=>o.measurement.window=null,o=>o.measurement.channel='wrong']) {
    await temporary(async(dir)=>{const doc=readDocument(root,'tests/fixtures/gold.json');change(doc.value.cases.ADP01['ADP01.01']);
      const run=await execute(bundle(),{root,dir,client:mockClient,fixtureDocument:doc});assert.equal(run.cases[0].product_verdict,'INCONCLUSIVE');});
  }
});
test('state rule catches physical inconsistency, illegal transitions, battery evidence and missing quality',()=>{
  const b=bundle(),s=b.cases.find(c=>c.id==='STATE01').checks[0];
  const o=readDocument(root,'tests/fixtures/gold.json').value.cases.STATE01['STATE01.01'];
  o.events.at(-1).physical_power_w=0;assert.equal(stateRule(s,o,b.states).product_verdict,'FAIL');
  o.events.at(-1).physical_power_w=null;assert.equal(stateRule(s,o,b.states).product_verdict,'INCONCLUSIVE');
  o.events=[o.events[0],{...o.events.at(-1),physical_power_w:5}];assert.equal(stateRule(s,o,b.states).reason_code,'STATE.ILLEGAL_TRANSITION');
  const battery={events:[{time_s:0,battery:'CC',quality:{issues:[]},evidence_refs:['x']}]};
  assert.equal(stateRule({layer:'battery',expected_final:'CC'},battery,b.states).product_verdict,'INCONCLUSIVE');
});
test('communication requires independent denominator, complete coverage and layer match',()=>{
  const b=bundle(),s=b.cases.find(c=>c.id==='COMM-QI').checks[0],o=readDocument(root,'tests/fixtures/gold.json').value.cases['COMM-QI']['COMM-QI.01'];
  for(const patch of [{expected_count:0},{reference_verified:false},{complete:false},{layer:'input'},{received_count:101},{error_count:-1}])assert.equal(communicationRule(s,{...o,...patch}).product_verdict,'INCONCLUSIVE');
});
test('aggregate: no empty PASS, no unapproved SKIP, FAIL retained in error/cancel',()=>{
  assert.equal(aggregate([]),'INCONCLUSIVE');assert.equal(aggregate([{product_verdict:'SKIP'}]),'INCONCLUSIVE');
  for(const state of ['error','cancelled','interrupted'])assert.equal(aggregate([{product_verdict:'FAIL'}],state),'FAIL');
});
test('standard result and run metadata preserve separate execution and product statuses',async()=>temporary(async(dir)=>{
  const r=normalizeResult({ok:true,execution_status:'completed',product_verdict:'FAIL',validation_verdict:'FAIL',industrial_approved:false,run_id:'run1',attempt_id:'attempt1'});
  assert.equal(r.ok,true);assert.equal(r.product_verdict,'FAIL');
  const file=writeRunJson({run_dir:dir,project:'test',stamp:'s'},{id:'wireless-charge-validation'},r);
  const doc=JSON.parse(fs.readFileSync(file));assert.equal(doc.product_verdict,'FAIL');assert.equal(doc.execution_status,'completed');assert.equal(doc.attempt_id,'attempt1');
}));
test('automation client polls with remaining deadline and no unknown resubmission',async()=>{
  const client=new AutomationClient(root);let calls=0;
  client.request=async(route,body,budget)=>{assert.equal(body.method,'job.get');assert(budget>0&&budget<=1000);calls++;return {state:'unknown'};};
  assert.equal((await client.wait('job',{timeoutMs:50})).state,'unknown');assert.equal(calls,1);
  await assert.rejects(client.wait('job',{timeoutMs:0}),/INVALID_WAIT_BUDGET/);
  const controller=new AbortController();controller.abort();await assert.rejects(client.wait('job',{signal:controller.signal}));assert.equal(calls,1);
});
test('late succeeded response cannot bypass a step deadline',async()=>temporary(async(dir)=>{
  const b=bundle();b.cases[0].deadline_ms=1;
  const client={async request(...args){await new Promise(r=>setTimeout(r,10));return mockClient.request(...args);}};
  const run=await execute(b,{root,dir,client});assert.equal(run.execution_status,'error');assert.equal(run.validation_verdict,'INCONCLUSIVE');assert(run.errors.some(e=>e.code==='STEP.DEADLINE'));
}));
test('reused stamp cannot overwrite an existing attempt and stop targets only active attempt',async()=>temporary(async(dir)=>{
  const testDir=path.join(dir,'test'),runDir=path.join(testDir,'runs','same-stamp');
  const ctx={pluginDir:root,plugin:readDocument(root,'plugin.json').value,args:{mode:'dry-run'},dataRoot:dir,layout:{test_dir:testDir,run_dir:runDir,project:'test',test_id:'WirelessCharge'}};
  const task=runPlugin(ctx);const active=JSON.parse(fs.readFileSync(path.join(testDir,'wireless.active.json')));
  assert.equal((await task).attempt_id,active.attempt);
  const file=path.join(runDir,'artifacts/reports/checks.json');const original=fs.readFileSync(file,'utf8');
  await assert.rejects(runPlugin(ctx),/EXISTING_ATTEMPT/);assert.equal(fs.readFileSync(file,'utf8'),original);
  fs.writeFileSync(path.join(testDir,'wireless.active.json'),JSON.stringify({attempt:'active-1'}));
  await stop(ctx);assert.equal(JSON.parse(fs.readFileSync(path.join(testDir,'wireless.stop.json'))).attempt,'active-1');
}));


test('generated Markdown links resolve to intact per-case and raw evidence files',async()=>temporary(async(dir)=>{
  const run=await execute(bundle(),{root,dir,client:mockClient});
  const text=fs.readFileSync(run.summary_md,'utf8');
  const links=[...text.matchAll(/\]\(([^)]+)\)/g)].map(m=>m[1]);
  assert.equal(links.length,run.cases.length+6);
  for(const link of links) assert(fs.statSync(path.join(dir,link)).isFile(),link);
  const record=JSON.parse(fs.readFileSync(path.join(dir,'case-001.json')));
  assert.equal(record.run_id,run.run_id);assert.equal(record.case_id,run.cases[0].case_id);
  assert.deepEqual(record.checks,run.cases[0].checks);
  const manifest=JSON.parse(fs.readFileSync(path.join(dir,'manifest.json')));
  for(const file of manifest.files) assert.equal(hash(fs.readFileSync(path.join(dir,file.path))),file.sha256);
}));
