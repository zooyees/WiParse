import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {evaluate} from '../../../test-tools/plugins/fpga-firmware-test/lib/rules.mjs';
import {validateCase} from '../../../test-tools/plugins/fpga-firmware-test/lib/config-loader.mjs';
import {resolveResources} from '../../../test-tools/plugins/fpga-firmware-test/lib/resource-resolver.mjs';
import {capture} from '../../../test-tools/plugins/fpga-firmware-test/lib/scheduler.mjs';
import {loadSchema} from '../../../test-tools/lib/hub/schema.mjs';
import run,{preflight} from '../../../test-tools/plugins/fpga-firmware-test/index.mjs';
import {REQUIRED_CAPABILITIES} from '../../../test-tools/plugins/fpga-firmware-test/lib/host-adapter.mjs';
const here=path.dirname(fileURLToPath(import.meta.url)),repo=path.resolve(here,'../../..'),plugin=path.join(repo,'test-tools/plugins/fpga-firmware-test');
const base=JSON.parse(fs.readFileSync(path.join(repo,'recipes/fpga-firmware-test/cases/01-pll_clock.json')));
const schema=loadSchema(path.join(plugin,'schemas/fpga-test-case.schema.json'),plugin);
const evidence={log:{complete:true,decoded:true,text:'unrelated log'}};
const results=[];
for(const rule of [{id:'empty',type:'all',required:true,rules:[]},{id:'missing_pattern',type:'log_regex',required:true}]){
 const c=structuredClone(base);c.verdict.rules=[rule];results.push({scenario:rule.id,schema_errors:schema.errors(c),cross_field_errors:validateCase(c),result:evaluate([rule],evidence)});
}
results.push({scenario:'wrong_uart_channel',selected:resolveResources({log:{kind:'wiparse_dongle_uart',selector:{dongle_serial:'D1',uart_channel:1,baud:115200},required:true}},[{kind:'wiparse_dongle_uart',dongle_serial:'D1',uart_channel:0,device_id:'channel-zero'}]).log});
const scratch=fs.mkdtempSync(path.join(repo,'tmp/fpga-review-repro-'));
try{
 let stopped=false,sourceCalls=0;const dir=path.join(scratch,'capture');fs.mkdirSync(dir);
 const bytes=Buffer.from('screenshot');const shot=path.join(dir,'shot.png');fs.writeFileSync(shot,bytes);const sha256=createHash('sha256').update(bytes).digest('hex');
 await capture({log_capture:{enabled:false},waveform_captures:[{id:'shot',at_s:0,channels:['CH1'],timeout_s:1,source_waveform:{enabled:true}}]}, {scope:{}},{capture:async()=>{stopped=true;return {screenshot:{path:shot,sha256}}},waveformSource:async()=>{sourceCalls++;return {artifact:{path:shot,sha256},metrics:{}}}}, {now:()=>0,sleep:async()=>{}},()=>stopped,{capture_t0:0},dir,{});
 results.push({scenario:'stop_after_screenshot',source_calls_after_stop:sourceCalls});
 const stalled=capture({log_capture:{enabled:false},waveform_captures:[{id:'stalled',at_s:0,channels:['CH1'],timeout_s:0.001,source_waveform:{enabled:false}}]}, {scope:{}},{capture:()=>new Promise(()=>{})}, {now:()=>0,sleep:async()=>{}},()=>false,{capture_t0:0},dir,{});
 const timeoutState=await Promise.race([stalled.then(()=> 'completed',()=> 'rejected'),new Promise(resolve=>setTimeout(()=>resolve('still_pending_after_40ms'),40))]);
 results.push({scenario:'capture_timeout_not_enforced',configured_timeout_ms:1,observation:timeoutState});
 const cases=path.join(scratch,'cases');fs.mkdirSync(cases);const fw=path.join(scratch,'fw.bin');fs.writeFileSync(fw,'fw');
 const c=structuredClone(base);c.id='case';c.name='Review fixture';c.dut={};c.timeout_s=10;c.waveform_captures=[];c.log_capture.duration_s=0.001;c.verdict.rules=[{id:'boot',type:'log_contains',text:'OK',required:true}];c.firmware.path='../fw.bin';c.firmware.sha256=createHash('sha256').update('fw').digest('hex');
 c.instruments={programmer:{kind:'debug_probe',selector:{serial:'D1'},required:true},log:{kind:'serial_port',selector:{port:'COM1',baud:115200},required:true}};fs.writeFileSync(path.join(cases,'case.json'),JSON.stringify(c));
 let acquired=0,released=0;
 const adapter={version:'wiparse.fpga-host/1',capabilities:REQUIRED_CAPABILITIES,devices:async()=>[{kind:'debug_probe',serial:'D1'},{kind:'serial_port',port:'COM1'}],preflight:async()=>{},storage:async()=>{},acquire:async()=>{acquired++;return {}},release:async()=>{released++},sourceOutput:async()=>{},voltage:async()=>12,probeReset:async()=>{},collectLog:async()=>({raw_base64:Buffer.from('OK').toString('base64'),lines:[],start_cursor:0,end_cursor:2,dropped_lines:0,window_covered:true,complete:true}),capture:async()=>{},waveformSource:async()=>{},cleanup:async()=>({ok:true})};
 const ctx={args:{config_dir:cases,dut_serial:'SN',board_revision:'A',fpga_model:'X',fixture_id:'F',station_id:'S',resume_mode:'fresh'},testDir:path.join(scratch,'test'),repositoryRoot:scratch,layout:{run_dir:path.join(scratch,'run')},fpgaAdapter:adapter};
 const pf=await preflight(ctx);if(!pf.ok)throw Error(JSON.stringify(pf));
 const successful=await run(ctx);
 ctx.previousFpgaRun=successful;ctx.args.resume_mode='unfinished';let resumeError;
 try{await run(ctx);}catch(e){resumeError=e.code;}
 results.push({scenario:'resume_original_manifest',error:resumeError});
 const pf2=await preflight(ctx);if(!pf2.ok)throw Error(JSON.stringify(pf2));
 try{await run(ctx);}catch(e){resumeError=e.code;}
 results.push({scenario:'resume_new_preflight',error:resumeError});
 ctx.args.resume_mode='fresh';delete ctx.previousFpgaRun;await preflight(ctx);
 const before={acquired,released};fs.writeFileSync(path.join(ctx.testDir,'matrix.json'),'{broken');let matrixError;
 try{await run(ctx);}catch(e){matrixError=e.message;}
 results.push({scenario:'matrix_parse_after_lease',acquired:acquired-before.acquired,released:released-before.released,error:matrixError});
 const files=fs.readdirSync(path.join(repo,'recipes/fpga-firmware-test/cases')).filter(f=>f.endsWith('.json'));
 results.push({scenario:'demo_filename_contract',total:files.length,mismatches:files.filter(f=>JSON.parse(fs.readFileSync(path.join(repo,'recipes/fpga-firmware-test/cases',f))).id!==f.slice(0,-5)).length});
}finally{fs.rmSync(scratch,{recursive:true,force:true});}
fs.writeFileSync(path.join(here,'reproduction-results.json'),JSON.stringify(results,null,2)+'\n');
console.log(JSON.stringify(results,null,2));

