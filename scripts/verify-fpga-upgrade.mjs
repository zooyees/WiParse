/** End-to-end upgrade acceptance with the 20 shipped IP cases in an isolated data root. */
import fs from 'node:fs';import path from 'node:path';import {fileURLToPath} from 'node:url';import {spawn} from 'node:child_process';import http from 'node:http';import {execute} from '../test-tools/hub-workspace.mjs';
const repo=fileURLToPath(new URL('..',import.meta.url)),root=path.resolve(process.argv[2]??`tmp/fpga-upgrade-demo-${Date.now()}`),recipe=path.join(root,'recipes/fpga-firmware-test'),cases=path.join(recipe,'cases');
if(fs.existsSync(cases))throw Error('Choose a new verification directory');fs.mkdirSync(cases,{recursive:true});fs.cpSync(path.join(repo,'recipes/fpga-firmware-test/firmware'),path.join(recipe,'firmware'),{recursive:true,errorOnExist:true});
const params={config_dir:'recipes/fpga-firmware-test/cases',execution_mode:'simulation',simulation_scenario:process.argv[3]??'pass',loop_count:1};
const context={pluginDir:path.join(repo,'test-tools/plugins/fpga-firmware-test'),dataRoot:root,project:'default',testId:'FpgaFirmwareTest',params};
const values=[];for(const filename of fs.readdirSync(path.join(repo,'recipes/fpga-firmware-test/cases')).filter(f=>f.endsWith('.json'))){const value=JSON.parse(fs.readFileSync(path.join(repo,'recipes/fpga-firmware-test/cases',filename)));value.log_capture.duration_s=0.1;value.waveform_captures.forEach(w=>{w.at_s=0.03;w.timeout_s=1;});value.timeout_s=5;fs.writeFileSync(path.join(cases,filename),JSON.stringify(value,null,2)+'\n');values.push(value);}
// Explicit migration is restricted to this newly created simulation workspace.
const migration=execute({context,internal:true,method:'project.migration.preview'});
fs.writeFileSync(path.join(root,'migration-preview.json'),JSON.stringify(migration,null,2));
if(!migration.ok||migration.conflicts.length)throw Error('Simulation migration conflicts: '+JSON.stringify(migration.conflicts));
execute({context,internal:true,method:'project.migration.commit',params:{plan_sha256:migration.plan_sha256}});
const before=execute({context,internal:true,method:'workspace.open'});if(before.items.length!==20||before.matrix.available)throw Error('Unrun repository must contain the 20 migrated business cases');
const server=http.createServer((req,res)=>{res.setHeader('Content-Type','application/json');res.end(JSON.stringify({ok:true,data:{version:fs.readFileSync(path.join(repo,'Cargo.toml'),'utf8').match(/^version\s*=\s*"([^"]+)"/m)[1]}}));});await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
const overlay=path.join(root,'overlay.json');
function runner(args){return new Promise((resolve,reject)=>{const child=spawn(process.execPath,[path.join(repo,'test-tools/runner.mjs'),'--plugin','fpga-firmware-test','--data-root',root,'--url',`http://127.0.0.1:${server.address().port}`,'--overlay',overlay,...args],{windowsHide:true});let output='';child.stdout.on('data',b=>output+=b);child.stderr.on('data',b=>output+=b);child.on('error',reject);child.on('close',code=>code===0?resolve(output):reject(Error(output)));});}
try{
 const allIds=values.map(c=>c.id),ids=allIds.slice(0,Math.max(1,Math.min(20,Number(process.argv[4])||20)));fs.writeFileSync(overlay,JSON.stringify({params,test_matrix:{selected_item_ids:ids}}));await runner(['--lifecycle','preflight']);
 const state=execute({context,internal:true,method:'preflight.state'});if(!state.valid)throw Error('Invalid preflight');
 fs.writeFileSync(overlay,JSON.stringify({params,test_matrix:{selected_item_ids:ids,config_manifest_sha256:state.manifest_sha256,preflight_revision:state.revision}}));
 await runner(['--lifecycle','run','--stamp','20261018_040000_upgrade']);
 const workspace=execute({context,internal:true,method:'workspace.open'}),record=execute({context,internal:true,method:'config.get',params:{config_id:ids[0]}});
 if(workspace.matrix.error||workspace.matrix.summary[params.simulation_scenario==='fail'?'fail':'pass']!==ids.length)throw Error(JSON.stringify(workspace.matrix));
 for(const id of [ids[0],ids[Math.floor(ids.length/2)],ids.at(-1)]){const row=workspace.matrix.items.find(r=>r.id===id);if(row?.result_ref){const detail=execute({context,internal:true,method:'result.get',params:{config_id:id,run_id:workspace.matrix.run_id,attempt:row.attempt}});if(!detail.available||detail.value.item_id!==id)throw Error('Incorrect result detail '+id);}}
 const report=path.join(root,'projects/default/tests/FpgaFirmwareTest/runs/20261018_040000_upgrade/artifacts/reports/summary.md');const links=[...fs.readFileSync(report,'utf8').matchAll(/\]\(([^)]+)\)/g)].map(m=>decodeURIComponent(m[1]));for(const link of links)if(!fs.existsSync(path.resolve(path.dirname(report),link)))throw Error('Broken link '+link);
 const first=workspace.matrix.items.find(r=>r.id===ids[0]);const details=execute({context,internal:true,method:'result.get',params:{config_id:first.id,run_id:workspace.matrix.run_id,attempt:first.attempt}});
 fs.writeFileSync(path.join(root,'fpga-ui-workspace.json'),JSON.stringify({params,workspace,record,details}));
 const verification={ok:true,run:'projects/default/tests/FpgaFirmwareTest/runs/20261018_040000_upgrade',simulation_only:true,item_count:values.length,selected_count:ids.length,project_config:workspace.project_config.sha256,valid_evidence_links:links.length,summary:workspace.matrix.summary};fs.writeFileSync(path.join(root,'verification.json'),JSON.stringify(verification,null,2));
 console.log(JSON.stringify({ok:true,simulation_only:true,count:values.length,summary:workspace.matrix.summary,valid_evidence_links:links.length,fixture:path.join(root,'fpga-ui-workspace.json')},null,2));
}finally{await new Promise(resolve=>server.close(resolve));}
