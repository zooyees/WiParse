import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {ConfigStore} from '../lib/hub/config-store.mjs';
import {workspace} from '../lib/hub/manifest.mjs';
import {getProject,saveProject,previewMigration,commitMigration} from '../lib/hub/project-config.mjs';
import {manifest,completePreflight,preflightState,beginRun,endRun} from '../lib/hub/run-binding.mjs';
import {readJson} from '../lib/hub/common.mjs';
import {resultDetails} from '../lib/hub/result-details.mjs';
const demo=fileURLToPath(new URL('../plugins/hub-workspace-demo',import.meta.url));
function setup(t){
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'hub-project-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  const dir=path.join(root,'plugins','fixture');fs.cpSync(demo,dir,{recursive:true});const plugin=readJson(path.join(dir,'plugin.json'));
  plugin.ui.project_config={protocol:'wiparse.project-config/1',schema:'project.schema.json'};
  fs.writeFileSync(path.join(dir,'project.schema.json'),JSON.stringify({type:'object',properties:{name:{type:'string',minLength:1},port:{type:'string',format:'serial-port'}},required:['name','port'],additionalProperties:false}));
  fs.writeFileSync(path.join(dir,'index.mjs'),`export function configure(ctx){return {lifecycle:'configure',ok:true,errors:[],warnings:[]};}
export function configureProject(ctx){return {lifecycle:'configure',ok:ctx.projectConfig.value.port!=='INVALID',errors:ctx.projectConfig.value.port==='INVALID'?[{path:'/port',message:'Invalid binding'}]:[],warnings:[]};}
export function planProjectMigration({cases,projectConfig}){return {ok:!cases.some(v=>v.name==='conflict'),conflicts:cases.some(v=>v.name==='conflict')?[{path:'port',ids:cases.map(v=>v.id)}]:[],project_value:projectConfig?.value??{name:'Migrated project',port:'COM3'},items:cases.every(v=>v.name.endsWith(' migrated'))?[]:cases.map(v=>({id:v.id,value:{...v,name:v.name+' migrated'}})),role_mapping:[{from:'log',to:'log_com'}]};}`);
  fs.writeFileSync(path.join(dir,'plugin.json'),JSON.stringify(plugin));
  const make=(project='first',testId='main',repository=null)=>new ConfigStore(workspace(repository?{...plugin,ui:{...plugin.ui,config_editor:{...plugin.ui.config_editor,repository}}}:plugin,{pluginDir:dir,dataRoot:path.join(root,'data'),project,testId}));
  return {root,dir,plugin,make,s:make()};
}
function save(s,value={name:'Project A',port:'COM3'}){const p=getProject(s);return saveProject(s,{value,expected_revision:p.revision,expected_sha256:p.sha256});}
function add(s,id='one'){const d=s.draft(id);return s.save({...d,expected_revision:0,expected_sha256:''});}
function preflight(s){const f=s.withLock(()=>manifest(s));completePreflight(s,f,{ok:true,test_matrix:{manifest_sha256:f.manifest_sha256,revision:f.manifest_sha256}});return f;}

test('project settings require explicit save, validate bindings and optimistic revision',t=>{
  const {s}=setup(t);add(s);assert.equal(getProject(s).saved,false);assert.throws(()=>manifest(s),/PROJECT.REQUIRED/);
  assert.throws(()=>save(s,{name:'A',port:'INVALID'}),/PROJECT.INVALID/);assert.equal(getProject(s).saved,false);
  const p=save(s);assert.equal(p.revision,1);assert.equal(p.sha256.length,64);
  assert.throws(()=>saveProject(s,{value:p.value,expected_revision:0,expected_sha256:''}),/REVISION_CONFLICT/);
});
test('project/test/plugin isolation and envelope integrity reject cross-scope or tampered data',t=>{
  const {s,make}=setup(t);save(s);assert.equal(getProject(make('second')).saved,false);assert.equal(getProject(make('first','other-test')).saved,false);
  const body=readJson(s.s.projectFile);body.value.port='COM99';fs.writeFileSync(s.s.projectFile,JSON.stringify(body));assert.throws(()=>getProject(s),/CONTENT_MISMATCH/);
});
test('shared changes invalidate preflight, freeze metadata and block active-run edits',t=>{
  const {s}=setup(t);add(s);save(s);const frozen=preflight(s);assert.equal(preflightState(s).valid,true);
  const binding=beginRun(s,['one'],path.join(s.s.testDir,'runs','one'));
  try{assert.equal(readJson(path.join(s.s.testDir,'runs','one','project-config.json')).sha256,frozen.project_config.sha256);assert.throws(()=>save(s,{name:'B',port:'COM4'}),/LOCKED_BY_RUN/);}finally{endRun(s,binding);}
  save(s,{name:'B',port:'COM4'});assert.equal(preflightState(s).valid,false);assert.notEqual(manifest(s).project_config.sha256,frozen.project_config.sha256);
});
test('project and multi-file migration journal rolls back failures and preserves exact backup',t=>{
  const {s}=setup(t);add(s);add(s,'two');const original=s.get('one').sha256,plan=previewMigration(s);
  s.fault=point=>{if(point==='after_replace')throw Error('injected');};assert.throws(()=>commitMigration(s,{plan_sha256:plan.plan_sha256}),/injected/);assert.equal(getProject(s).saved,false);assert.equal(s.get('one').sha256,original);
  s.fault=null;const result=commitMigration(s,{plan_sha256:plan.plan_sha256});assert.equal(result.migrated,2);assert.equal(getProject(s).value.port,'COM3');assert.match(s.get('one').value.name,/ migrated$/);
  const backup=readJson(path.join(path.dirname(s.s.projectFile),'backups','migration-'+plan.plan_sha256+'.json'));assert.equal(backup.items.length,2);assert.equal(JSON.parse(Buffer.from(backup.items[0].bytes,'base64')).name,'one');
  const again=previewMigration(s);assert.equal(commitMigration(s,{plan_sha256:again.plan_sha256}).migrated,0);
});
test('migration preview binds all old inputs and conflicting cases remain untouched',t=>{
  const {s}=setup(t);const a=add(s),plan=previewMigration(s);s.save({config_id:a.config_id,value:{...a.value,name:'conflict'},expected_revision:a.revision,expected_sha256:a.sha256});
  assert.throws(()=>commitMigration(s,{plan_sha256:plan.plan_sha256}),/REVISION_CONFLICT/);const conflict=previewMigration(s);assert.equal(conflict.conflicts.length,1);assert.throws(()=>commitMigration(s,{plan_sha256:conflict.plan_sha256}),/MIGRATION_CONFLICT/);assert.equal(getProject(s).saved,false);
});
test('same project with two repositories shares CAS, recovery and active-run guards',t=>{
  const {s,make}=setup(t),other=make('first','main','{data_root}/alternate-cases');
  assert.equal(s.s.editorRoot,other.s.editorRoot);assert.equal(s.s.projectFile,other.s.projectFile);
  add(s);add(other,'another');save(s);const original=getProject(other);
  s.fault=stage=>{if(stage==='after_replace'){assert.throws(()=>save(other,{name:'Concurrent',port:'COM9'}),/CONFIG.BUSY/);throw Error('injected');}};
  assert.throws(()=>save(s,{name:'Interrupted',port:'COM4'}),/injected/);assert.equal(getProject(other).sha256,original.sha256);
  s.fault=stage=>{if(stage==='before_replace')throw Error('crash');};assert.throws(()=>save(s),/crash/);other.withLock(()=>{});s.fault=null;assert.equal(getProject(other).sha256,original.sha256);
  save(other,{name:'Other repository write',port:'COM8'});assert.throws(()=>saveProject(s,{value:original.value,expected_revision:original.revision,expected_sha256:original.sha256}),/REVISION_CONFLICT/);
  preflight(s);const binding=beginRun(s,['one'],path.join(s.s.testDir,'runs','live'));
  try{assert.throws(()=>save(other),/LOCKED_BY_RUN/);const p=previewMigration(other);assert.throws(()=>commitMigration(other,{plan_sha256:p.plan_sha256}),/LOCKED_BY_RUN/);}finally{endRun(s,binding);}
});
test('result details verify scope, both attempts, item and shared hashes; old files are unverified',t=>{
  const {s}=setup(t);s.s.plugin.ui.result_details={protocol:'wiparse.result-details/1',schema_version:'fixture.result/2'};
  const shared={revision:1,sha256:'b'.repeat(64),schema_hash:'c'.repeat(64)},uuid='attempt-current';
  const ref='tests/main/runs/one/artifacts/'+uuid+'/result.json',file=path.join(s.s.dataRoot,'projects',s.s.project,ref);fs.mkdirSync(path.dirname(file),{recursive:true});
  const matrix={schema_version:'wiparse.test-matrix/1',plugin_id:s.s.plugin.id,run_id:'run-current',revision:1,updated_at:new Date().toISOString(),preflight_revision:'manifest',project_config:shared,summary:{total:1,selected:1,pending:0,running:0,pass:1,fail:0,error:0,skipped:0,aborted:0,inconclusive:0},items:[{id:'one',name:'One',tags:[],selected:true,status:'pass',phase:'complete',attempt:2,attempt_id:uuid,config_sha256:'a'.repeat(64),progress:1,message:'',result_ref:ref}]};
  fs.writeFileSync(s.s.matrix,JSON.stringify(matrix));
  const value={schema_version:'fixture.result/2',scope:{project_id:s.s.project,test_id:'main',plugin_id:s.s.plugin.id},item_id:'one',run_id:'run-current',attempt:2,attempt_id:uuid,config_sha256:'a'.repeat(64),project_config:shared};
  const request={config_id:'one',run_id:'run-current',attempt:2};
  fs.writeFileSync(file,JSON.stringify(value));assert.equal(resultDetails(s,request).available,true);
  for(const changes of [{item_id:'other'},{attempt:1},{attempt_id:'old-uuid'},{config_sha256:'e'.repeat(64)},{project_config:{...shared,sha256:'e'.repeat(64)}},{scope:{...value.scope,test_id:'different'}}]){fs.writeFileSync(file,JSON.stringify({...value,...changes}));assert.throws(()=>resultDetails(s,request),/CONTENT_MISMATCH/);}
  fs.writeFileSync(file,'{"product_verdict":"PASS"}');assert.equal(resultDetails(s,request).error,'RESULT.UNVERIFIABLE_HISTORY');assert.throws(()=>resultDetails(s,{...request,attempt:1}),/STALE_SELECTION/);
});
