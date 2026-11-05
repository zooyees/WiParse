import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import run, {preflight} from '../index.mjs';
import {hash} from '../lib/common.mjs';
import {REQUIRED_CAPABILITIES} from '../lib/host-adapter.mjs';
import {prepareProjectFixture} from './fixtures/project-fixture.mjs';
import {preserveLog} from '../lib/log-capture.mjs';
import {evaluate} from '../lib/rules.mjs';

function fixture(t) {
  const root=fs.mkdtempSync(path.join(process.cwd(),'fpga-review-'));
  t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  const cases=path.join(root,'cases');fs.mkdirSync(cases);fs.writeFileSync(path.join(root,'fw.bin'),'fw');
  for(const id of ['boot','second']) {
    const c={schema_version:'wiparse.fpga-test-case/1',id,name:id,enabled:true,order:id==='boot'?0:1,timeout_s:4,dut:{},
      instruments:{programmer:{kind:'debug_probe',selector:{serial:'D1'},required:true},log:{kind:'serial_port',selector:{port:'COM1',baud:115200},required:true}},
      firmware:{download:false,path:'../fw.bin',sha256:hash('fw'),format:'bin',target:'FPGA',base_address:0},
      reset:{method:'wiparse_dongle',dongle_role:'programmer',hardware:true},timing:{anchor:'after_reset_release'},
      log_capture:{enabled:true,start_at_s:0,duration_s:0.001,max_lines:100,encoding:'utf-8'},waveform_captures:[],
      verdict:{mode:'all',rules:[{id:'ok',type:'log_contains',text:'OK',required:true}]}};
    fs.writeFileSync(path.join(cases,id+'.json'),JSON.stringify(c));
  }
  let text='BAD',acquisitions=0;
  const adapter={version:'wiparse.fpga-host/1',capabilities:REQUIRED_CAPABILITIES,
    devices:async()=>[{kind:'debug_probe',serial:'D1'},{kind:'serial_port',port:'COM1'}],
    preflight:async()=>{},storage:async()=>{},acquire:async()=>{acquisitions++;return {};},release:async()=>{},
    sourceOutput:async()=>{},voltage:async()=>12,probeReset:async()=>{},capture:async()=>{},waveformSource:async()=>{},
    cleanup:async()=>({ok:true}),cancelAndConfirm:async()=>({quiescent:true}),
    collectLog:async()=>({raw_base64:Buffer.from(text).toString('base64'),lines:[{text}],start_cursor:0,end_cursor:text.length,dropped_lines:0,window_covered:true,complete:true})};
  const testDir=path.join(root,'test');
  const ctx={testDir,repositoryRoot:root,args:{config_dir:cases,resume_mode:'fresh',dut_serial:'SN',board_revision:'A',fpga_model:'FPGA',fixture_id:'F',station_id:'S'},
    layout:{run_dir:path.join(testDir,'runs','one')},fpgaAdapter:adapter};
  prepareProjectFixture(ctx);
  return {root,cases,ctx,adapter,setText:value=>text=value,acquisitions:()=>acquisitions};
}

test('selected preflight failure blocks entire plan without silently deselecting the bad case',async t=>{
  const f=fixture(t);
  f.adapter.preflight=async({config})=>{if(config.id==='second')throw Object.assign(new Error('device unavailable'),{code:'RESOURCE.MISSING'});};
  const result=await preflight(f.ctx);
  assert.equal(result.ok,false);assert(result.checks.some(c=>c.code==='RESOURCE.MISSING'));
  assert(!fs.existsSync(path.join(f.ctx.testDir,'fpga-manifest.json')));
  const matrix=JSON.parse(fs.readFileSync(path.join(f.ctx.testDir,'matrix.json')));
  assert.equal(matrix.items.find(i=>i.id==='second').selected,true);
  assert.equal(matrix.items.find(i=>i.id==='second').status,'error');
  await assert.rejects(run(f.ctx),/PREFLIGHT_REQUIRED/);assert.equal(f.acquisitions(),0);
});

test('invalid repository file cannot be silently ignored while valid cases pass',async t=>{
  const f=fixture(t);fs.writeFileSync(path.join(f.cases,'broken.json'),'{');
  assert.equal((await preflight(f.ctx)).ok,false);
  await assert.rejects(run(f.ctx),/PREFLIGHT_REQUIRED/);
});

for(const kind of ['unknown','disabled'])test(`${kind} selection from host cannot be executed`,async t=>{
  const f=fixture(t);
  if(kind==='disabled') {const p=path.join(f.cases,'second.json'),c=JSON.parse(fs.readFileSync(p));c.enabled=false;fs.writeFileSync(p,JSON.stringify(c));}
  f.ctx.testMatrix={selected_item_ids:['boot',kind==='disabled'?'second':'missing'],manifest_sha256:'test'};
  const result=await preflight(f.ctx);assert.equal(result.ok,false);
  assert(result.checks.some(c=>c.code===(kind==='disabled'?'RUN.DISABLED_SELECTION':'RUN.UNKNOWN_SELECTION')));
  assert.equal(f.acquisitions(),0);
});

test('failure stop exposes unexecuted coverage and evidence reasons in offline report',async t=>{
  const f=fixture(t);f.ctx.args.stop_on_fail=true;f.ctx.args.loop_count=2;
  assert.equal((await preflight(f.ctx)).ok,true);
  const suite=await run(f.ctx);assert.equal(suite.product_verdict,'FAIL');assert.equal(suite.stop_reason,'stop_on_fail');
  assert.equal(suite.coverage[0].completed,1);assert.equal(suite.coverage[1].completed,0);assert.equal(suite.coverage[1].planned,2);
  const report=fs.readFileSync(suite.summary_md,'utf8');
  assert.match(report,/执行覆盖/);assert.match(report,/失败即停/);assert.match(report,/未执行项不等于通过/);assert.match(report,/rule_not_satisfied/);
  const item=JSON.parse(fs.readFileSync(path.resolve(path.dirname(suite.summary_md),suite.results[0].result_ref)));
  assert.equal(item.diagnostics.first_rule_issue.id,'ok');assert.equal(item.diagnostics.automatic_retry,false);
});

test('relative recovery path is based on the test directory rather than process cwd',async t=>{
  const f=fixture(t);assert.equal((await preflight(f.ctx)).ok,true);const first=await run(f.ctx);
  f.ctx.args.resume_mode='failed';f.ctx.args.resume_from='runs/one';f.ctx.layout.run_dir=path.join(f.ctx.testDir,'runs','two');f.setText('OK');
  assert.equal((await preflight(f.ctx)).ok,true);const second=await run(f.ctx);
  assert.equal(second.product_verdict,'PASS');assert.equal(second.parent_run_id,first.run_id);
});

test('malformed base64 is rejected and bad measurement quality/unit have specific reasons',t=>{
  const f=fixture(t);
  assert.throws(()=>preserveLog({raw_base64:'T0s=garbage',lines:[]},f.root,{max_lines:100}),/LOG.RAW_ENCODING_INVALID/);
  const rule={id:'frequency',type:'waveform_metric_range',capture:'ping',channel:'CH1',metric:'frequency_hz',unit:'Hz',minimum:1,maximum:2,required:true};
  for(const [metric,reason] of [[{value:1,unit:'Hz',quality:'bad'},'measurement_quality_invalid'],[{value:1,unit:'kHz',quality:'good'},'measurement_unit_mismatch']]) {
    const result=evaluate([rule],{metrics:{ping:{CH1:{frequency_hz:metric}}}});
    assert.equal(result.product_verdict,'INCONCLUSIVE');assert.equal(result.results[0].reason,reason);
  }
});

test('schema generator retains published project and evidence capabilities',t=>{
  const root=fs.mkdtempSync(path.join(process.cwd(),'fpga-generator-'));
  t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  fs.mkdirSync(path.join(root,'test-tools/plugins/fpga-firmware-test/schemas'),{recursive:true});
  const plugin=fileURLToPath(new URL('..',import.meta.url));
  const generated=spawnSync(process.execPath,[path.join(plugin,'generate-schemas.mjs')],{cwd:root,encoding:'utf8'});
  assert.equal(generated.status,0,generated.stderr);
  assert.deepEqual(JSON.parse(fs.readFileSync(path.join(root,'test-tools/plugins/fpga-firmware-test/plugin.json'))),
    JSON.parse(fs.readFileSync(path.join(plugin,'plugin.json'))));
});

test('measurement capability is checked before acquiring devices or resetting DUT',async t=>{
  const f=fixture(t),file=path.join(f.cases,'boot.json'),c=JSON.parse(fs.readFileSync(file));
  c.verdict.rules=[{id:'supply',type:'instrument_measure_range',required:true,resource_role:'programmer',metric:'mean_v',minimum:11,maximum:13,unit:'V'}];
  fs.writeFileSync(file,JSON.stringify(c));
  const result=await preflight(f.ctx);assert.equal(result.ok,false);
  assert(result.checks.some(c=>c.code==='HOST.MEASUREMENT_REQUIRED'));assert.equal(f.acquisitions(),0);
});
