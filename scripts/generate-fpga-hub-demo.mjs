/** Reproducible five-case, explicitly simulated FPGA evidence under a chosen data root. */
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
import {spawn} from 'node:child_process';
import http from 'node:http';
import {execute} from '../test-tools/hub-workspace.mjs';
const repo=fileURLToPath(new URL('..',import.meta.url));
const root=path.resolve(process.argv[2]||'tmp/fpga-hub-demo');
const recipe=path.join(root,'recipes/fpga-firmware-test'),cases=path.join(recipe,'cases');
fs.mkdirSync(cases,{recursive:true});fs.mkdirSync(path.join(recipe,'firmware'),{recursive:true});
if(fs.readdirSync(cases).some(n=>n.endsWith('.json')))throw Error('Choose a fresh demo data root; existing cases are never overwritten');
const firmware=Buffer.from('WIPARSE SIMULATION FIRMWARE - NOT A PROGRAMMABLE PRODUCT IMAGE\n');
fs.writeFileSync(path.join(recipe,'firmware/simulation.bin'),firmware);
const digest=createHash('sha256').update(firmware).digest('hex');
const params={config_dir:'recipes/fpga-firmware-test/cases',execution_mode:'simulation',simulation_scenario:process.argv[3]||'pass',loop_count:1};
const pluginDir=path.join(repo,'test-tools/plugins/fpga-firmware-test');
const context={pluginDir,dataRoot:root,project:'default',testId:'FpgaFirmwareTest',params};
const values=Array.from({length:5},(_,index)=>({
  schema_version:'wiparse.fpga-test-case/1',id:`sim_${index+1}`,name:`模拟验证 ${index+1}`,enabled:true,order:index,tags:['simulation',index<3?'smoke':'regression'],timeout_s:5,
  dut:{},instruments:{programmer:{kind:'debug_probe',selector:{serial:'SIM-PROBE'},required:true},scope:{kind:'oscilloscope',selector:{serial:'SIM-SCOPE'},required:true},log:{kind:'serial_port',selector:{port:'SIM-COM',baud:115200},required:true}},
  firmware:{download:false,path:'../firmware/simulation.bin',sha256:digest,target:'SIMULATED FPGA',format:'bin',base_address:0},
  reset:{method:'wiparse_dongle',dongle_role:'programmer',hardware:true,release_confirmation:{type:'command_complete',timeout_ms:100}},timing:{anchor:'after_reset_release'},
  log_capture:{enabled:true,start_at_s:0,duration_s:0.1,max_lines:100,encoding:'utf-8'},
  waveform_captures:[{id:'clock',at_s:0.03,kind:'screenshot',channels:['CH1'],source_waveform:{enabled:true,format:'csv'},timeout_s:1}],
  verdict:{mode:'all',rules:[{id:'boot',type:'log_contains',text:'BOOT_OK',required:true},{id:'frequency',type:'waveform_metric_range',capture:'clock',channel:'CH1',metric:'frequency_hz',minimum:99000,maximum:101000,unit:'Hz',required:true}],ai_assist:{enabled:false,policy:'advisory_only'}}
}));
for(const value of values)fs.writeFileSync(path.join(cases,value.id+'.json'),JSON.stringify(value,null,2)+'\n');
const migration=execute({context,internal:true,method:'project.migration.preview'});if(!migration.ok||migration.conflicts.length)throw Error('Simulation migration conflicts');execute({context,internal:true,method:'project.migration.commit',params:{plan_sha256:migration.plan_sha256}});
const server=http.createServer((req,res)=>{res.setHeader('Content-Type','application/json');res.end(JSON.stringify({ok:true,data:{version:fs.readFileSync(path.join(repo,'Cargo.toml'),'utf8').match(/^version\s*=\s*"([^"]+)"/m)[1]}}));});
await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
function run(args){return new Promise((resolve,reject)=>{const child=spawn(process.execPath,[path.join(repo,'test-tools/runner.mjs'),'--plugin','fpga-firmware-test','--data-root',root,'--url',`http://127.0.0.1:${server.address().port}`,...args],{windowsHide:true});let output='';child.stdout.on('data',b=>output+=b);child.stderr.on('data',b=>output+=b);child.on('error',reject);child.on('close',code=>code===0?resolve(output):reject(Error(output)));});}
const overlay=path.join(root,'simulation-overlay.json');
const selection=values.slice(0,Math.max(1,Math.min(5,Number(process.argv[4])||5))).map(v=>v.id);
try{
  fs.writeFileSync(overlay,JSON.stringify({params,test_matrix:{selected_item_ids:selection}}));
  const preflight=await run(['--lifecycle','preflight','--overlay',overlay]);
  const state=execute({context,internal:true,method:'preflight.state'});
  if(!state.valid)throw Error('Preflight binding invalid');
  fs.writeFileSync(overlay,JSON.stringify({params,test_matrix:{config_manifest_sha256:state.manifest_sha256,preflight_revision:state.revision,selected_item_ids:state.selected_item_ids}}));
  const output=await run(['--lifecycle','run','--stamp','20261018_150000_simulation','--overlay',overlay]);
  const workspace=execute({context,internal:true,method:'workspace.open'}),record=execute({context,internal:true,method:'config.get',params:{config_id:'sim_1'}});
  if(workspace.matrix.error||workspace.matrix.summary[params.simulation_scenario==='fail'?'fail':'pass']!==selection.length)throw Error(JSON.stringify(workspace.matrix));
  fs.writeFileSync(path.join(root,'fpga-ui-workspace.json'),JSON.stringify({params,workspace,record}));
  fs.writeFileSync(path.join(root,'verification.json'),JSON.stringify({ok:true,simulation_only:true,count:5,summary:workspace.matrix.summary,preflight,runner:output},null,2));
  console.log(JSON.stringify({ok:true,data_root:root,simulation_only:true,count:5,summary:workspace.matrix.summary}));
}finally{await new Promise(resolve=>server.close(resolve));}
