import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {performance} from 'node:perf_hooks';
import {hash} from '../lib/common.mjs';
import {ruleErrors} from '../lib/rule-contract.mjs';
import {evaluate} from '../lib/rules.mjs';
import {resolveResources} from '../lib/resource-resolver.mjs';
import {capture} from '../lib/scheduler.mjs';
import {guardedAdapter} from '../lib/io-guard.mjs';
import {REQUIRED_CAPABILITIES} from '../lib/host-adapter.mjs';
import {loadSchema} from '../../../lib/hub/schema.mjs';
import {advise} from '../lib/ai-adapter.mjs';
import run,{preflight as pluginPreflight} from '../index.mjs';
import {prepareProjectFixture,caseDocument} from './fixtures/project-fixture.mjs';
const preflight=ctx=>{prepareProjectFixture(ctx);return pluginPreflight(ctx);};
const plugin=fileURLToPath(new URL('..',import.meta.url));
function fixture(t){
 const root=fs.mkdtempSync(path.join(process.cwd(),'fpga-upgrade-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
 const cases=path.join(root,'cases');fs.mkdirSync(cases);fs.writeFileSync(path.join(root,'fw.bin'),'fw');
 const c={schema_version:'wiparse.fpga-test-case/1',id:'boot',name:'启动检查',enabled:true,dut:{},timeout_s:4,instruments:{programmer:{kind:'debug_probe',selector:{serial:'D1'},required:true},log:{kind:'serial_port',selector:{port:'COM1',baud:115200},required:true}},firmware:{download:false,path:'../fw.bin',sha256:hash('fw'),target:'FPGA',format:'bin',base_address:0},reset:{method:'wiparse_dongle',dongle_role:'programmer',hardware:true,release_confirmation:{type:'command_complete',timeout_ms:100}},timing:{anchor:'after_reset_release'},log_capture:{enabled:true,start_at_s:0,duration_s:0.001,max_lines:100,encoding:'utf-8'},waveform_captures:[],verdict:{mode:'all',rules:[{id:'boot_ok',type:'log_contains',required:true,text:'OK'}]}};
 const save=()=>fs.writeFileSync(path.join(cases,'boot.json'),JSON.stringify(c));save();
 const a={version:'wiparse.fpga-host/1',capabilities:REQUIRED_CAPABILITIES,devices:async()=>[{kind:'debug_probe',serial:'D1'},{kind:'serial_port',port:'COM1'}],preflight:async()=>{},storage:async()=>{},acquire:async()=>({lease:1}),release:async()=>{},sourceOutput:async()=>{},voltage:async()=>12,probeReset:async()=>{},capture:async()=>{},waveformSource:async()=>{},cleanup:async()=>({ok:true}),cancelAndConfirm:async()=>({quiescent:true}),collectLog:async()=>({raw_base64:Buffer.from('OK').toString('base64'),lines:[{text:'OK'}],start_cursor:0,end_cursor:2,dropped_lines:0,window_covered:true,complete:true})};
 const testDir=path.join(root,'test');
 const ctx={testDir,repositoryRoot:root,args:{config_dir:cases,resume_mode:'fresh',dut_serial:'SN',board_revision:'A',fpga_model:'FPGA',fixture_id:'FIX',station_id:'STATION'},layout:{run_dir:path.join(testDir,'runs','one')},fpgaAdapter:a};
 return {root,c,cases,save,a,ctx};
}
test('F01 malformed rule branches are rejected by schema and defensive verdict',t=>{
 const {c}=fixture(t),schema=loadSchema(path.join(plugin,'schemas/fpga-test-case.schema.json'),plugin);
 for(const r of [{id:'empty',type:'all',required:true,rules:[]},{id:'regex',type:'log_regex',required:true}]){
  c.verdict.rules=[r];assert(schema.errors(caseDocument(c)).length>0);assert(ruleErrors(r).length>0);assert.notEqual(evaluate([r],{log:{text:'unrelated',complete:true,decoded:true}}).product_verdict,'PASS');
 }
 assert(ruleErrors({id:'r',type:'log_regex',required:true,pattern:'a*'}).length>0);
});
test('F04 exact Dongle UART channel is required',()=>{
 assert.throws(()=>resolveResources({log:{kind:'wiparse_dongle_uart',selector:{dongle_serial:'D1',uart_channel:1},required:true}},[{kind:'wiparse_dongle_uart',dongle_serial:'D1',uart_channel:0}]),/RESOURCE.MISSING/);
});
test('F03 Stop between screenshot and source never starts source',async t=>{
 const {root}=fixture(t);let stop=false,calls=0;const shot=path.join(root,'s.png');fs.writeFileSync(shot,'shot');
 await assert.rejects(capture({id:'case',log_capture:{enabled:false},waveform_captures:[{id:'s',at_s:0,source_waveform:{enabled:true}}]},{scope:{}},{capture:async()=>{stop=true;return {screenshot:{path:shot,sha256:hash('shot')}}},waveformSource:async()=>{calls++;}}, {now:()=>0,sleep:async()=>{}},()=>stop,{capture_t0:0},root,{}),/RUN.ABORTED/);
 assert.equal(calls,0);
});
test('F02 hung I/O is bounded and cancellation must confirm quiescence',async()=>{
 let unknown=false,cancelled=0;const clock={now:()=>performance.now()};
 let io=guardedAdapter({capture:()=>new Promise(()=>{}),cancelAndConfirm:async()=>{cancelled++;return {quiescent:true};}},{clock,stopped:()=>false,deadline:clock.now()+20});
 await assert.rejects(io.capture({config:{timeout_s:0.01}}),/IO.TIMEOUT/);assert.equal(cancelled,1);
 io=guardedAdapter({capture:()=>new Promise(()=>{})},{clock,stopped:()=>false,deadline:clock.now()+20,onUnknown:()=>unknown=true});
 await assert.rejects(io.capture({config:{timeout_s:0.01}}),/IO.STATE_UNKNOWN/);assert.equal(unknown,true);
});
test('F05 matrix is validated before a lease, and post-acquire file failure releases once',async t=>{
 const {ctx,a}=fixture(t);let acquisitions=0,releases=0;a.acquire=async()=>{acquisitions++;return {};};a.release=async()=>{releases++;};assert.equal((await preflight(ctx)).ok,true);
 fs.writeFileSync(path.join(ctx.testDir,'matrix.json'),'{broken');await assert.rejects(run(ctx));assert.equal(acquisitions,0);
 await preflight(ctx); // a corrupt prior matrix explicitly blocks preflight; remove and retry.
 fs.unlinkSync(path.join(ctx.testDir,'matrix.json'));assert.equal((await preflight(ctx)).ok,true);
 a.acquire=async()=>{acquisitions++;fs.mkdirSync(path.join(ctx.layout.run_dir,'artifacts/reports/summary.md'),{recursive:true});return {};};
 const result=await run(ctx);assert.equal(result.execution_status,'error');assert.equal(releases,1);
});
test('F06 failed retry uses immutable content identity and a new linked attempt',async t=>{
 const {ctx,a}=fixture(t);a.collectLog=async()=>({raw_base64:Buffer.from('bad').toString('base64'),lines:[],start_cursor:0,end_cursor:3,dropped_lines:0,window_covered:true,complete:true});
 assert.equal((await preflight(ctx)).ok,true);const first=await run(ctx);assert.equal(first.product_verdict,'FAIL');
 ctx.args.resume_mode='failed';ctx.args.resume_from=path.join(ctx.layout.run_dir,'artifacts/reports/suite-result.json');ctx.layout.run_dir=path.join(ctx.testDir,'runs','two');
 a.collectLog=async()=>({raw_base64:Buffer.from('OK').toString('base64'),lines:[],start_cursor:0,end_cursor:2,dropped_lines:0,window_covered:true,complete:true});
 const pf=await preflight(ctx);assert.equal(pf.ok,true,JSON.stringify(pf));const second=await run(ctx);assert.equal(second.product_verdict,'PASS');assert.equal(second.parent_run_id,first.run_id);assert.equal(second.results[0].previous_attempt_id,first.results[0].attempt_id);assert.notEqual(second.results[0].attempt_id,first.results[0].attempt_id);
});
test('F08 archive failure leaves ERROR, and preserves known product FAIL',async t=>{
 const {ctx,a}=fixture(t);a.collectLog=async()=>({raw_base64:Buffer.from('bad').toString('base64'),lines:[],start_cursor:0,end_cursor:3,dropped_lines:0,window_covered:true,complete:true});
 a.cleanup=async()=>{const items=path.join(ctx.layout.run_dir,'artifacts/items/boot');const attempt=fs.readdirSync(items)[0];fs.mkdirSync(path.join(items,attempt,'result.json'));return {ok:true};};
 assert.equal((await preflight(ctx)).ok,true);const result=await run(ctx);assert.equal(result.execution_status,'error');assert.equal(result.product_verdict,'FAIL');const m=JSON.parse(fs.readFileSync(path.join(ctx.testDir,'matrix.json')));assert.equal(m.items[0].status,'error');assert.equal(m.active_item_id,null);
});
test('F11 nested measurement rules compile to acquisition dependencies',async t=>{
 const {ctx,a,c,save}=fixture(t);let calls=0;
 c.verdict.rules=[{id:'group',type:'all',required:true,rules:[{id:'supply',type:'instrument_measure_range',resource_role:'programmer',metric:'mean_v',minimum:11,maximum:13,unit:'V',required:true}]}];save();
 a.measure=async()=>{calls++;return {value:12,unit:'V',quality:'good'};};assert.equal((await preflight(ctx)).ok,true);assert.equal((await run(ctx)).product_verdict,'PASS');assert.equal(calls,1);
});
test('F12 required AI skipped by external policy cannot keep PASS',async t=>{
 const {ctx,c,save}=fixture(t);c.verdict.ai_assist={enabled:true,required:true,policy:'advisory_only',provider_profile:'public',redaction_profile:'safe',inputs:['rule_results']};save();ctx.args.ai_global_enable=true;ctx.fpgaStation={ai_profiles:{public:{kind:'public',adapter:'approved',model:'model',credential_ref:'secret-store:key',allow_fields:['rule_results']}}};
 assert.equal((await preflight(ctx)).ok,true);assert.equal((await run(ctx)).product_verdict,'INCONCLUSIVE');
});
test('F12 AI sends the intersection of case inputs and provider allowlist',async()=>{
 let payload;await advise({enabled:true,provider_profile:'p',inputs:['rule_results']},{allow_public_ai:true,ai_profiles:{p:{kind:'public',model:'m',allow_fields:['rule_results','waveform_metrics']}}},{ai:async v=>{payload=v.payload;return {recommendation:'REVIEW',confidence:1};}},{metrics:{CH1:'private'}},{results:[]});assert.deepEqual(Object.keys(payload),['rule_results']);
});
import {pauseBoundary} from '../lib/run-control.mjs';
import {controlState,requestControl} from '../../../lib/hub/run-control.mjs';
test('pause is acknowledged only at a safe item boundary, continue starts the next item',async t=>{
 const {ctx,c,cases,a}=fixture(t);fs.writeFileSync(path.join(cases,'second.json'),JSON.stringify({...c,id:'second',name:'Second'}));ctx.runId='pause-run';let resets=0;a.probeReset=async()=>{resets++;};
 const log=a.collectLog;a.collectLog=async()=>{if(resets===1)fs.writeFileSync(path.join(ctx.testDir,'fpga-control.json'),JSON.stringify({run_id:ctx.runId,action:'pause',state:'pause_requested'}));return log();};
 assert.equal((await preflight(ctx)).ok,true);const running=run(ctx),control=path.join(ctx.testDir,'fpga-control.json');let paused=false;
 for(let i=0;i<200;i++){await new Promise(r=>setTimeout(r,10));if(fs.existsSync(control)&&JSON.parse(fs.readFileSync(control)).state==='paused'){paused=true;break;}}
 assert(paused);assert.equal(resets,1);const snapshot=JSON.parse(fs.readFileSync(path.join(ctx.testDir,'matrix.json')));assert.equal(snapshot.items[1].status,'pending');
 fs.writeFileSync(control,JSON.stringify({run_id:ctx.runId,action:'resume',state:'resume_requested'}));const result=await running;assert.equal(result.product_verdict,'PASS');assert.equal(resets,2);
});
test('Stop remains effective while paused and never resets a DUT',async t=>{
 const {ctx,a}=fixture(t);ctx.runId='pause-before-first';let resets=0;a.probeReset=async()=>resets++;
 assert.equal((await preflight(ctx)).ok,true);const control=path.join(ctx.testDir,'fpga-control.json');fs.writeFileSync(control,JSON.stringify({run_id:ctx.runId,action:'pause',state:'pause_requested'}));
 const running=run(ctx);for(let i=0;i<200;i++){await new Promise(r=>setTimeout(r,5));if(JSON.parse(fs.readFileSync(control)).state==='paused')break;}
 fs.writeFileSync(path.join(ctx.testDir,'fpga.stop'),'stop');const result=await running;assert.equal(result.execution_status,'cancelled');assert.equal(resets,0);
});
test('generic run control rejects stale run IDs and invalid transitions',t=>{
 const {root}=fixture(t);const store={s:{testDir:root,controlFile:path.join(root,'control.json')},runLocks:()=>({run_id:'active'})};
 assert.throws(()=>requestControl(store,{run_id:'stale',action:'pause'}),/RUN.CONTROL_STALE/);
 assert.equal(requestControl(store,{run_id:'active',action:'pause'}).state,'pause_requested');
 assert.throws(()=>requestControl(store,{run_id:'active',action:'pause'}),/RUN.CONTROL_INVALID_STATE/);
 assert.equal(requestControl(store,{run_id:'active',action:'resume'}).state,'resume_requested');assert.equal(controlState(store).run_id,'active');
});
import {evaluateBounded} from '../lib/rule-executor.mjs';
test('pathological regex cannot block the runner or Stop',async()=>{
 const started=performance.now();await assert.rejects(evaluateBounded([{id:'expensive',type:'log_regex',required:true,pattern:'^(a+)+$'}],{log:{complete:true,decoded:true,text:'a'.repeat(8000)+'!'}},{timeoutMs:100}),/RULE.TIMEOUT/);assert(performance.now()-started<3000);
});
test('crash recovery prefers a newer interrupted attempt over an older completed index',async t=>{
 const {ctx}=fixture(t);assert.equal((await preflight(ctx)).ok,true);const first=await run(ctx),oldDir=ctx.layout.run_dir;
 fs.appendFileSync(path.join(oldDir,'artifacts/docs/attempt-started.jsonl'),JSON.stringify({item_id:'boot',attempt_id:'interrupted-attempt',execution_status:'running',started_at:'2099-01-01T00:00:00Z'})+'\n');
 fs.writeFileSync(path.join(oldDir,'artifacts/docs/run-state.json'),JSON.stringify({run_id:first.run_id,status:'error'}));
 ctx.args.resume_mode='unfinished';ctx.args.resume_from=oldDir;ctx.layout.run_dir=path.join(ctx.testDir,'runs','recovery');
 const pf=await preflight(ctx);assert.equal(pf.ok,true,JSON.stringify(pf));const resumed=await run(ctx);assert.equal(resumed.product_verdict,'PASS');assert.equal(resumed.results[0].previous_attempt_id,'interrupted-attempt');
});
