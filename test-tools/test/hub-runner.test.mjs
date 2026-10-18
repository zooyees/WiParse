import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import os from 'node:os';
import http from 'node:http';
import {spawn} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {execute} from '../hub-workspace.mjs';
const root=fileURLToPath(new URL('../..',import.meta.url));
function launch(args,onOutput=()=>{}){return new Promise((resolve,reject)=>{const child=spawn(process.execPath,[path.join(root,'test-tools/runner.mjs'),...args],{windowsHide:true});let output='';const receive=b=>{output+=b;onOutput(String(b));};child.stdout.on('data',receive);child.stderr.on('data',receive);child.on('error',reject);child.on('close',code=>resolve({code,output}));});}
test('runner configure has no project side effects; preflight/run freeze and stale gating',async t=>{
 const dataRoot=fs.mkdtempSync(path.join(os.tmpdir(),'wiparse-hub-runner-'));t.after(()=>fs.rmSync(dataRoot,{recursive:true,force:true}));
 const pluginDir=path.join(root,'test-tools/plugins/hub-workspace-demo'),context={pluginDir,dataRoot,project:'default',testId:'HubWorkspaceDemo',params:{}};
 const overlay=path.join(dataRoot,'overlay.json');fs.writeFileSync(overlay,JSON.stringify({config:{id:'candidate'}}));
 const server=http.createServer((req,res)=>{res.setHeader('Content-Type','application/json');res.end(JSON.stringify({ok:true,version:'1.2.0',data:{version:'1.2.0'}}));});await new Promise(r=>server.listen(0,'127.0.0.1',r));t.after(()=>server.close());
 const base=['--plugin','hub-workspace-demo','--data-root',dataRoot,'--url',`http://127.0.0.1:${server.address().port}`];
 const configure=await launch([...base,'--lifecycle','configure','--overlay',overlay]);assert.equal(configure.code,0,configure.output);assert.equal(fs.existsSync(path.join(dataRoot,'projects')),false);
 const draft=execute({context,method:'config.create',params:{config_id:'a'},internal:true});execute({context,method:'config.save',params:{...draft,expected_revision:0,expected_sha256:''},internal:true});
 // Force actual inter-process contention, releasing only after the runner has
 // observed BUSY; this is not a sleep-based guess about startup timing.
 const {ConfigStore}=await import('../lib/hub/config-store.mjs');const {workspace}=await import('../lib/hub/manifest.mjs');
 const plugin=JSON.parse(fs.readFileSync(path.join(pluginDir,'plugin.json')));const store=new ConfigStore(workspace(plugin,context));
 const mutex=path.join(store.s.editorRoot,'write.lock');fs.mkdirSync(mutex);fs.writeFileSync(path.join(mutex,'owner.json'),JSON.stringify({pid:process.pid,nonce:'runner-contention-test'}));
 const stopFile=path.join(store.s.testDir,'run.stop');fs.mkdirSync(store.s.testDir,{recursive:true});fs.writeFileSync(stopFile,'previous stop remains acknowledged only by a new launcher start');
 let observed=false,preflight;
 try {preflight=await launch([...base,'--lifecycle','preflight'],chunk=>{if(chunk.includes('CONFIG.BUSY_WAIT')){observed=true;fs.rmSync(mutex,{recursive:true});}});}finally{fs.rmSync(mutex,{recursive:true,force:true});}
 assert.equal(observed,true,'Runner did not exercise the acquisition wait');assert.equal(preflight.code,0,preflight.output);
 assert.equal(fs.existsSync(stopFile),true,'Preflight must not delete a prior stop');
 const state=execute({context,method:'preflight.state',internal:true});assert.equal(state.valid,true,JSON.stringify(state));
 const binding={config_manifest_sha256:state.manifest_sha256,preflight_revision:state.revision,selected_item_ids:['a']};fs.writeFileSync(overlay,JSON.stringify({params:{},test_matrix:binding}));
 const retainedStop=await launch([...base,'--lifecycle','run','--stamp','retained_stop','--overlay',overlay]);assert.notEqual(retainedStop.code,0);assert.match(retainedStop.output,/RUN.ABORTED/);assert.equal(fs.existsSync(path.join(store.s.testDir,'runs/retained_stop')),false);
 fs.unlinkSync(stopFile); // Explicit new-start preparation by the launcher.
 fs.mkdirSync(mutex);fs.writeFileSync(path.join(mutex,'owner.json'),JSON.stringify({pid:process.pid,nonce:'run-contention-stop-test'}));
 let waitingStop=false,cancelled;
 try {cancelled=await launch([...base,'--lifecycle','run','--stamp','stopped_during_lock','--overlay',overlay],chunk=>{if(chunk.includes('CONFIG.BUSY_WAIT')){waitingStop=true;fs.writeFileSync(stopFile,'new stop while the launch is waiting');}});}finally{fs.rmSync(mutex,{recursive:true,force:true});}
 assert.equal(waitingStop,true);assert.notEqual(cancelled.code,0);assert.match(cancelled.output,/RUN.ABORTED/);assert.equal(fs.existsSync(path.join(store.s.testDir,'runs/stopped_during_lock')),false,'Cancelled wait must not enter plugin execution');assert.equal(fs.existsSync(stopFile),true);
 fs.unlinkSync(stopFile); // Explicit new start after cancellation.
 const run=await launch([...base,'--lifecycle','run','--stamp','test_run','--overlay',overlay]);assert.equal(run.code,0,run.output);
 const runJson=JSON.parse(fs.readFileSync(path.join(dataRoot,'projects/default/tests/HubWorkspaceDemo/runs/test_run/run.json')));assert.equal(runJson.test_matrix.manifest_sha256,state.manifest_sha256);assert.deepEqual(runJson.test_matrix.selected_item_ids,['a']);
 const matrix=execute({context,method:'matrix.get',internal:true});assert.equal(matrix.summary.pass,1);
 const row=execute({context,method:'config.get',params:{config_id:'a'},internal:true});execute({context,method:'config.save',params:{config_id:'a',value:{...row.value,name:'changed'},expected_revision:row.revision,expected_sha256:row.sha256},internal:true});
 const stale=await launch([...base,'--lifecycle','run','--stamp','forbidden','--overlay',overlay]);assert.notEqual(stale.code,0);assert.match(stale.output,/PREFLIGHT/);assert.equal(fs.existsSync(path.join(dataRoot,'projects/default/tests/HubWorkspaceDemo/runs/forbidden')),false);
});

test('plugin endpoint inherits GUI bind without overriding explicit user server',async()=>{
 const {resolveApiUrl}=await import('../lib/wiparse-sdk.mjs');
 assert.equal(resolveApiUrl(null,{WIPARSE_API_BIND:'127.0.0.1:48237'}),'http://127.0.0.1:48237');
 assert.equal(resolveApiUrl(null,{WIPARSE_API_BIND:'0.0.0.0:48237'}),'http://127.0.0.1:48237');
 assert.equal(resolveApiUrl(null,{WIPARSE_API_BIND:'[::]:48237'}),'http://[::1]:48237');
 assert.equal(resolveApiUrl('https://station.example/',{WIPARSE_URL:'http://server',WIPARSE_API_BIND:'127.0.0.1:1'}),'https://station.example');
 assert.equal(resolveApiUrl(null,{WIPARSE_URL:'http://server',WIPARSE_API_BIND:'127.0.0.1:1'}),'http://server');
 assert.equal(resolveApiUrl(null,{}),'http://127.0.0.1:7878');
});
