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
test('live local gate reports the blocker without contacting a service',async()=>temporary(async(dir)=>{
  let calls=0;
  const client={invoke(){calls++;throw Error('unexpected');},request(){calls++;throw Error('unexpected');}};
  const result=await execute(bundle('live'),{root,dir,client});
  assert.equal(calls,0);
  assert.equal(result.execution_status,'error');
  assert.equal(result.product_verdict,'INCONCLUSIVE');
  assert(result.errors.some(e=>e.code.includes('INTEGRATION.TX_RX_FT4222_GATE_NOT_IMPLEMENTED')));
  assert(fs.readFileSync(result.summary_md,'utf8').includes('INTEGRATION.TX_RX_FT4222_GATE_NOT_IMPLEMENTED'));
}));
for(const [fixture,expected] of [['gold','PASS'],['bad','FAIL'],['missing','INCONCLUSIVE'],['quality','INCONCLUSIVE']]) {
  test(`${fixture} replay validates all SYS and never certifies a DUT`,async()=>temporary(async(dir)=>{
    const run=await execute(bundle('replay',fixture),{root,dir,client:mockClient});
    assert.equal(run.execution_status,'completed');assert.equal(run.validation_verdict,expected);assert.equal(run.product_verdict,'INCONCLUSIVE');
    const diagnostic=JSON.parse(fs.readFileSync(path.join(dir,'investigation.json')));
    assert.equal(diagnostic.findings.length,run.cases.flatMap(c=>c.checks).filter(c=>c.product_verdict!=='PASS').length);
    const source=JSON.parse(fs.readFileSync(path.join(dir,diagnostic.replay.file)));
    for(const finding of diagnostic.findings){
      assert.equal(finding.confirmed_root_cause,null);
      if(finding.observation_pointer) assert(source.cases[finding.case_id][finding.check_id]);
    }
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
  assert(links.includes("investigation.json"));
  assert(links.includes("adapter-matrix.json"));
  assert.equal(links.length,run.cases.length+8);
  for(const link of links) assert(fs.statSync(path.join(dir,link)).isFile(),link);
  const record=JSON.parse(fs.readFileSync(path.join(dir,'case-001.json')));
  assert.equal(record.run_id,run.run_id);assert.equal(record.case_id,run.cases[0].case_id);
  assert.deepEqual(record.checks,run.cases[0].checks);
  const manifest=JSON.parse(fs.readFileSync(path.join(dir,'manifest.json')));
  for(const file of manifest.files) assert.equal(hash(fs.readFileSync(path.join(dir,file.path))),file.sha256);
}));

test('compare two archived runs and reject damaged input',async()=>temporary(async(dir)=>{
  const {compareRuns}=await import('../plugins/wireless-charge-validation/lib/compare-runs.mjs');
  const left=path.join(dir,'before'),right=path.join(dir,'after');
  await execute(bundle('replay','gold'),{root,dir:left,client:mockClient});
  await execute(bundle('replay','bad'),{root,dir:right,client:mockClient});
  const comparison=compareRuns(left,right);
  assert(comparison.rows.some(r=>r.comparable && r.before==='PASS' && r.after==='FAIL'));
  assert(comparison.rows.some(r=>Number.isFinite(r.delta)));
  fs.appendFileSync(path.join(right,'checks.json'),' ');
  assert.throws(()=>compareRuns(left,right),/HASH_MISMATCH/);
}));

test('project recording preserves raw evidence and does not send instrument or evaluation commands',async()=>temporary(async(dir)=>{
  const b=loadBundle(root,{mode:'recorded',capture_id:'bench1'});
  fs.mkdirSync(path.join(dir,'captures'));
  const data={schema_version:'wiparse.station-observation/1',simulation_only:false,
    station_id:'test-station',dut_serial:'test-unit',firmware:'test-build',clock_domain:'scope-1',
    cases:{ADP01:{'ADP01.01':{evidence_refs:['e1']}}},
    evidence:{e1:{source:'trace',coordinate:'sample_0based',position:0,raw:'1.03'}}};
  fs.writeFileSync(path.join(dir,'captures/bench1.json'),JSON.stringify(data));
  const result=await execute(b,{root,dir:path.join(dir,'result'),captureRoot:dir,client:{request(){throw Error('unexpected call');}}});
  assert.equal(result.execution_status,'completed');
  assert.equal(result.cases[0].checks[0].reason_code,'CAPTURE.OBSERVED_REVIEW_REQUIRED');
  assert.equal(result.product_verdict,'INCONCLUSIVE');
  assert.equal(result.cases[1].checks[0].reason_code,'CAPTURE.RAW_EVIDENCE_MISSING');
  const saved=JSON.parse(fs.readFileSync(path.join(dir,'result/recorded-evidence.json')));
  assert.equal(saved.evidence[saved.observed[0].evidence_refs[0]].raw,'1.03');
  assert(fs.readFileSync(result.summary_md,'utf8').includes('(recorded-evidence.json)'));
  assert.throws(()=>loadBundle(root,{mode:'recorded',capture_id:'../test'}),/INVALID_ID/);
}));

test('unverified live entry rejects before requesting a device',async()=>{
  const {captureCase}=await import('../plugins/wireless-charge-validation/lib/live.mjs');
  const b=bundle('live');
  let calls=0;
  await assert.rejects(captureCase(b,b.cases[0],{run_id:'r',attempt_id:'a',dut:{serial:'x',firmware:'y'}},async()=>{calls++;},()=>{}),/INTEGRATION.TX_RX_FT4222_GATE_NOT_IMPLEMENTED/);
  assert.equal(calls,0);
});

// Retain these intended live-path checks, but do not report them as passed
// while the external per-run RX-to-I2C execution gate is unavailable.
test.skip('BLOCKED: cancelled live stimulus is never retried; safe-stop feedback precedes lease release',async()=>{
  const {captureCase}=await import('../plugins/wireless-charge-validation/lib/live.mjs');
  const b=bundle('live');b.station.live={resource:'commissioning-fixture'};
  const calls=[];let cancelled=false;
  const request=async(method,p)=>{
    calls.push(method);
    if(method==='device.lease.acquire')return {resource:p.resource,lease_token:'fence',session_epoch:'epoch'};
    if(method==='capture.arm')return {...p,capture_id:'capture'};
    if(method==='dongle.capture.start')return {...p,ready:true};
    if(method==='capture.status')return {...p,armed:true,protection_healthy:true,watchdog_active:true,safe_before_stimulus:true};
    if(method==='stimulus.apply'){cancelled=true;return {...p,feedback_verified:true};}
    if(method==='station.safe_stop')return {...p,outputs_safe:true,feedback_verified:true};
    if(method==='device.lease.release')return {released:true};
    throw Error(method);
  };
  await assert.rejects(captureCase(b,b.cases[0],{run_id:'r',attempt_id:'a',dut:{serial:'x',firmware:'y'}},request,()=>{if(cancelled)throw Error('RUN.CANCELLED');}),/RUN.CANCELLED/);
  assert.deepEqual(calls.slice(-3),['stimulus.apply','station.safe_stop','device.lease.release']);
  assert.equal(calls.filter(c=>c==='stimulus.apply').length,1);
});

test.skip('BLOCKED: commissioned station contract archives evidence and evaluates the measured case',async()=>temporary(async(dir)=>{
  const b=bundle('live'),scenario=b.cases[0];b.cases=[scenario];b.args.dut_serial='test-dut';b.args.dut_firmware='test-fw';
  b.station.live={resource:'fixture'};b.station.station_id='contract-fixture';b.station.g4_approved=false;b.suite.industrial=false;
  b.limits.approved=true;b.limits.simulation_only=false;scenario.capture_plan.timing_budget_approved=true;
  b.station.protection={verified:true,stop_sequence_approved:true};b.station.bindings=Object.fromEntries(scenario.roles.map(k=>[k,{verified:true}]));
  const caps={implemented:[...scenario.required_capabilities,'device.lease.release'],live_contract:'wiparse.fenced-wireless/1',station_id:b.station.station_id,approved_station_sha256:b.hashes.station,watchdog_verified:true};
  const observations=readDocument(root,'tests/fixtures/gold.json').value.cases[scenario.id];
  const evidence={};
  for(const o of Object.values(observations))for(const k of o.measurement.evidence_refs){
    const raw=JSON.stringify(o.measurement.value);evidence[k]={source:'fixture-data',coordinate:'sample_0based',position:0,capture_id:'C',raw,sha256:hash(raw)};
  }
  let armed=false;
  const client={async invoke(){return caps;},async request(route,{method,params:p}){
    const identity=Object.fromEntries(['case_id','run_id','attempt_id','station_id','dut_serial','firmware'].map(k=>[k,p[k]]));
    switch(method){
      case 'device.lease.acquire':return {resource:p.resource,lease_token:'token',session_epoch:'epoch'};
      case 'capture.arm':return {...identity,capture_id:'C'};
      case 'dongle.capture.start':return {...identity,capture_id:'C',ready:true};
      case 'capture.status':if(armed)return {...identity,capture_id:'C',protection_healthy:true,state:'complete',simulation_only:false,quality_validated:true,observations,evidence};
        return {...identity,capture_id:'C',armed:true,watchdog_active:true,protection_healthy:true,safe_before_stimulus:true};
      case 'stimulus.apply':armed=true;return {...identity,capture_id:'C',command_id:p.command_id,feedback_verified:true};
      case 'station.safe_stop':return {...identity,outputs_safe:true,feedback_verified:true};
      case 'device.lease.release':return {released:true};
      default:return mockClient.request(route,{method,params:p});
    }
  }};
  const run=await execute(b,{root,dir,client});assert.equal(run.execution_status,'completed',JSON.stringify(run.errors));assert.equal(run.validation_verdict,'PASS');
  assert.equal(run.product_verdict,'INCONCLUSIVE');assert.equal(run.station_activity.at(-1).method,'device.lease.release');
  assert.equal(JSON.parse(fs.readFileSync(path.join(dir,'replay-input.json'))).captures[scenario.id].capture_id,'C');
}));
