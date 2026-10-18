import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {hash} from '../lib/common.mjs';
import {discover,validateCase} from '../lib/config-loader.mjs';
import {evaluate} from '../lib/rules.mjs';
import {advise} from '../lib/ai-adapter.mjs';
import {REQUIRED_CAPABILITIES} from '../lib/host-adapter.mjs';
import run,{preflight as pluginPreflight} from '../index.mjs';
import {prepareProjectFixture} from './fixtures/project-fixture.mjs';
const preflight=ctx=>{prepareProjectFixture(ctx);return pluginPreflight(ctx);};
function fixture(t) {
 const root=fs.mkdtempSync(path.join(process.cwd(),'fpga-test-')); t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
 const cases=path.join(root,'cases');fs.mkdirSync(cases);fs.writeFileSync(path.join(root,'fw.bin'),'firmware');
 const c={schema_version:'wiparse.fpga-test-case/1',id:'boot',name:'Boot',enabled:true,timeout_s:10,dut:{},instruments:{programmer:{kind:'debug_probe',selector:{serial:'one'},required:true},log:{kind:'serial_port',selector:{port:'COM1',baud:115200},required:true}},firmware:{download:false,path:'../fw.bin',sha256:hash('firmware'),format:'bin',target:'FPGA',base_address:0},reset:{method:'wiparse_dongle',dongle_role:'programmer',hardware:true},timing:{anchor:'after_reset_release'},log_capture:{enabled:true,start_at_s:0,duration_s:0.01,max_lines:100,encoding:'utf-8'},waveform_captures:[],verdict:{mode:'all',rules:[{id:'boot',type:'log_contains',text:'BOOT_OK',required:true}]}};
 const save=()=>fs.writeFileSync(path.join(cases,'boot.json'),JSON.stringify(c));save();
 return {root,cases,c,save};
}
test('discovery ignores hidden/disabled files, rejects 501 and download',t=>{
 const f=fixture(t);fs.writeFileSync(path.join(f.cases,'_ignore.json'),'{');assert.equal(discover(f.cases,f.root).items.length,1);
 f.c.firmware.download=true;f.save();assert.equal(discover(f.cases,f.root).errors[0].code,'FEATURE.FIRMWARE_DOWNLOAD_NOT_IMPLEMENTED');
 for(let i=0;i<500;i++)fs.writeFileSync(path.join(f.cases,`case${i}.json`),'{}');assert.throws(()=>discover(f.cases,f.root),/MAX_ITEMS/);
});
test('source overlap blocks, touching windows permitted',t=>{
 const {c}=fixture(t);c.timeout_s=100;c.waveform_captures=[{id:'a',at_s:0,timeout_s:10,channels:['CH1'],source_waveform:{enabled:true}},{id:'b',at_s:5,timeout_s:10,channels:['CH1']}];
 assert(validateCase(c).some(e=>e.code==='WAVEFORM.SCHEDULE_OVERLAP'));c.waveform_captures[1].at_s=10;assert(!validateCase(c).some(e=>e.code==='WAVEFORM.SCHEDULE_OVERLAP'));
});
test('required FAIL dominates missing evidence; truncated logs never PASS',()=>{
 const rules=[{id:'a',type:'log_contains',text:'OK',required:true},{id:'b',type:'waveform_metric_range',capture:'a',channel:'CH1',metric:'frequency_hz',minimum:1,maximum:2,unit:'Hz',required:true}];
 assert.equal(evaluate(rules,{log:{complete:true,decoded:true,text:'bad'}}).product_verdict,'FAIL');
 assert.equal(evaluate(rules,{log:{complete:true,decoded:true,text:'OK',truncated:true}}).product_verdict,'INCONCLUSIVE');
});
test('public AI permission gates transport and payload excludes identity/log',async()=>{
 let called=0;const config={enabled:true,provider_profile:'p'},station={ai_profiles:{p:{kind:'public',model:'m',allow_fields:['rule_results']}}},adapter={ai:async({payload})=>{called++;assert(!('log' in payload));return {recommendation:'PASS',confidence:1};}};
 assert.equal((await advise(config,station,adapter,{log:'secret'}, {results:[]})).status,'skipped');assert.equal(called,0);
 station.allow_public_ai=true;assert.equal((await advise(config,station,adapter,{log:'secret'}, {results:[]})).status,'completed');assert.equal(called,1);
});
test('mock host end-to-end freezes evidence and creates independent cycle attempts',async t=>{
 const f=fixture(t);let releases=0,resets=0;
 const adapter={version:'wiparse.fpga-host/1',capabilities:REQUIRED_CAPABILITIES,devices:async()=>[{kind:'debug_probe',serial:'one',device_id:'p'},{kind:'serial_port',port:'COM1',device_id:'l'}],preflight:async()=>{},storage:async()=>{},acquire:async()=>({lease:'one'}),release:async()=>releases++,sourceOutput:async()=>{},voltage:async()=>12,probeReset:async()=>resets++,collectLog:async()=>({raw_base64:Buffer.from('BOOT_OK').toString('base64'),lines:[{mono_ms:0,text:'BOOT_OK'}],start_cursor:1,end_cursor:2,dropped_lines:0,window_covered:true,complete:true}),capture:async()=>{},waveformSource:async()=>{},cleanup:async()=>({ok:true})};
 const ctx={args:{config_dir:f.cases,dut_serial:'SN',board_revision:'A',fpga_model:'FPGA',fixture_id:'FIX',station_id:'S',loop_count:2},testDir:path.join(f.root,'test'),layout:{run_dir:path.join(f.root,'run')},repositoryRoot:f.root,fpgaAdapter:adapter};
 const pf=await preflight(ctx);assert.equal(pf.ok,true,JSON.stringify(pf));const result=await run(ctx);assert.equal(result.product_verdict,'PASS');assert.equal(result.industrial_approved,false);assert.equal(result.results.length,2);assert.notEqual(result.results[0].attempt_id,result.results[1].attempt_id);assert.equal(resets,2);assert.equal(releases,1);assert(fs.existsSync(result.summary_md));
 f.c.name='changed';f.save();await assert.rejects(run(ctx),/CONFIG.CHANGED/);
});
test('missing host adapter fails closed',async t=>{const f=fixture(t);const r=await preflight({testDir:f.root,args:{config_dir:f.cases,dut_serial:'SN',board_revision:'A',fpga_model:'X',fixture_id:'F',station_id:'S'}});assert.equal(r.ok,false);assert.equal(r.checks[0].code,'HOST.ADAPTER_REQUIRED');});


test('stop during an attempt records ABORTED and never starts next item',async t=>{
 const f=fixture(t);const c2={...f.c,id:'second',name:'Second'};fs.writeFileSync(path.join(f.cases,'second.json'),JSON.stringify(c2));let resets=0,cleanup=0;
 const ctx={args:{config_dir:f.cases,dut_serial:'SN',board_revision:'A',fpga_model:'FPGA',fixture_id:'FIX',station_id:'S'},testDir:path.join(f.root,'test'),layout:{run_dir:path.join(f.root,'run')},repositoryRoot:f.root};
 ctx.fpgaAdapter={version:'wiparse.fpga-host/1',capabilities:REQUIRED_CAPABILITIES,devices:async()=>[{kind:'debug_probe',serial:'one'},{kind:'serial_port',port:'COM1'}],preflight:async()=>{},storage:async()=>{},acquire:async()=>({}),release:async()=>{},sourceOutput:async()=>{},voltage:async()=>12,probeReset:async()=>{resets++;fs.writeFileSync(path.join(ctx.testDir,'fpga.stop'),'stop');},collectLog:async()=>{throw new Error('must not collect');},capture:async()=>{},waveformSource:async()=>{},cleanup:async()=>{cleanup++;return {ok:true};}};
 assert.equal((await preflight(ctx)).ok,true);const result=await run(ctx);assert.equal(result.execution_status,'cancelled');assert.equal(result.product_verdict,'INCONCLUSIVE');assert.equal(resets,1);assert.equal(cleanup,1);const matrix=JSON.parse(fs.readFileSync(path.join(ctx.testDir,'matrix.json')));assert.equal(matrix.items[0].status,'aborted');assert.equal(matrix.items[1].status,'pending');
});
test('valid 500 configurations retain deterministic natural order',t=>{
 const f=fixture(t);for(let i=1;i<500;i++) fs.writeFileSync(path.join(f.cases,`item-${i}.json`),JSON.stringify({...f.c,id:`item-${i}`,name:`Case ${i}`}));const m=discover(f.cases,f.root);assert.equal(m.errors.length,0);assert.equal(m.items.length,500);assert.equal(m.items.findIndex(i=>i.config.name==='Case 2')<m.items.findIndex(i=>i.config.name==='Case 10'),true);
});
