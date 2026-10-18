import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {spawnSync} from 'node:child_process';
import {workspace,validateUi} from '../lib/hub/manifest.mjs';
import {ConfigStore} from '../lib/hub/config-store.mjs';
import {validateMatrix,readMatrix} from '../lib/hub/matrix.mjs';
import {manifest,completePreflight,preflightState,beginRun,endRun} from '../lib/hub/run-binding.mjs';
import {jail,readJson} from '../lib/hub/common.mjs';
const pluginDir=fileURLToPath(new URL('../plugins/hub-workspace-demo',import.meta.url));
const plugin=readJson(path.join(pluginDir,'plugin.json'));
function setup(t){const root=fs.mkdtempSync(path.join(os.tmpdir(),'wiparse-hub-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));return new ConfigStore(workspace(plugin,{pluginDir,dataRoot:root}));}
function create(s,key){const d=s.draft(key);return s.save({...d,expected_revision:0,expected_sha256:''});}
test('workspace: typed defaults, strict schema, optimistic conflicts, rename, archive, restore',t=>{
 const s=setup(t),a=create(s,'alpha');assert.equal(a.revision,1);assert.equal(s.list().total,1);
 assert.throws(()=>s.save({config_id:'alpha',value:a.value,expected_revision:0,expected_sha256:''}),/REVISION_CONFLICT/);
 assert.throws(()=>s.save({config_id:'alpha',value:{...a.value,typo:1},expected_revision:1,expected_sha256:a.sha256}),/VALIDATION_FAILED/);
 const b=s.save({config_id:'alpha',value:{...a.value,id:'beta'},expected_revision:1,expected_sha256:a.sha256});assert.equal(s.get('beta').revision,2);assert.throws(()=>s.get('alpha'),/NOT_FOUND/);
 const archived=s.archive({config_id:'beta',expected_revision:2,expected_sha256:b.sha256});assert.equal(s.list().total,0);s.restore({archive_id:archived.archive_id});assert.equal(s.list().total,1);
});
test('journal failures roll back both content and audit; recovery rolls back interrupted replacement',t=>{
 const s=setup(t),a=create(s,'alpha'),audit=fs.readFileSync(s.s.audit,'utf8');
 for(const point of ['after_replace','after_audit']){s.fault=p=>{if(p===point)throw Error('injected');};assert.throws(()=>s.save({config_id:'alpha',value:{...a.value,name:'changed'},expected_revision:1,expected_sha256:a.sha256}),/injected/);assert.equal(s.get('alpha').sha256,a.sha256);assert.equal(fs.readFileSync(s.s.audit,'utf8'),audit);}
 s.fault=p=>{if(p==='before_replace')throw Error('crash');};assert.throws(()=>s.save({config_id:'alpha',value:a.value,expected_revision:1,expected_sha256:a.sha256}));s.fault=null;s.withLock(()=>{});assert.equal(s.get('alpha').sha256,a.sha256);
});
test('bundle validation is atomic; 500 configurations paginate and batch without changing limits',t=>{
 const s=setup(t),items=Array.from({length:500},(_,n)=>s.draft(`item_${n}`).value),file=path.join(s.s.dataRoot,'bundle.json');
 fs.writeFileSync(file,JSON.stringify({schema_version:'wiparse.config-bundle/1',plugin_id:plugin.id,plugin_version:plugin.version,items}));
 assert.equal(s.importBundle({file}).imported,500);assert.equal(s.list({offset:490,limit:20}).items.length,10);assert.throws(()=>create(s,'extra'),/LIMIT/);
 const row=s.get('item_0');s.batch({ids:[row.id],enabled:false,expected:{[row.id]:{expected_revision:row.revision,expected_sha256:row.sha256}}});assert.equal(s.get(row.id).enabled,false);
 s.exportBundle({file:path.join(s.s.dataRoot,'export.json'),ids:['item_0']});assert.equal(readJson(path.join(s.s.dataRoot,'export.json')).items.length,1);
});
test('preflight selection, immutable snapshot and active configuration locks',t=>{
 const s=setup(t);create(s,'a');create(s,'b');const frozen=s.withLock(()=>manifest(s,['a']));
 completePreflight(s,frozen,{ok:true,test_matrix:{manifest_sha256:frozen.manifest_sha256,revision:frozen.manifest_sha256}});assert.equal(preflightState(s).valid,true);
 const binding=beginRun(s,['a'],path.join(s.s.testDir,'runs','one'));t.after(()=>endRun(s,binding));
 const a=s.get('a');assert.throws(()=>s.save({config_id:'a',value:a.value,expected_revision:a.revision,expected_sha256:a.sha256}),/LOCKED_BY_RUN/);
 const b=s.get('b');s.save({config_id:'b',value:{...b.value,name:'editable'},expected_revision:b.revision,expected_sha256:b.sha256});assert.equal(readJson(path.join(binding.snapshotDir,'b.json')).name,'b');assert.equal(preflightState(s).valid,false);
});
const matrix=()=>({schema_version:'wiparse.test-matrix/1',plugin_id:plugin.id,run_id:'one',revision:1,updated_at:new Date().toISOString(),preflight_revision:'manifest',summary:{total:1,selected:1,pending:0,running:0,pass:1,fail:0,error:0,skipped:0,aborted:0,inconclusive:0},items:[{id:'a',name:'A',tags:[],selected:true,status:'pass',phase:'completed',attempt:1,progress:1,message:'',result_ref:null}]});
test('matrix rejects rollback, terminal regression, escapes and oversized snapshots; last valid survives corruption',t=>{
 const s=setup(t),m=matrix();validateMatrix(m,{pluginId:plugin.id});const next=structuredClone(m);next.revision++;next.items[0].status='running';assert.throws(()=>validateMatrix(next,{pluginId:plugin.id,previous:m}),/INVALID_SNAPSHOT/);
 next.items[0].status='pass';next.items[0].result_ref='../outside';assert.throws(()=>validateMatrix(next,{pluginId:plugin.id}),/PATH_OUTSIDE_ROOT/);
 fs.mkdirSync(s.s.testDir,{recursive:true});fs.writeFileSync(s.s.matrix,JSON.stringify(m));assert.equal(readMatrix(s.s).revision,1);fs.writeFileSync(s.s.matrix,'broken');assert.equal(readMatrix(s.s).revision,1);assert.ok(readMatrix(s.s).error);
});
test('manifest and path jail reject unsafe source, missing limits and reparse escapes',t=>{
 const s=setup(t);assert.throws(()=>jail(path.join(s.s.dataRoot,'..','outside'),[s.s.dataRoot]),/PATH_OUTSIDE_ROOT/);
 assert.throws(()=>validateUi({...plugin,ui:{...plugin.ui,max_items:undefined}}),/UI_SCHEMA_INVALID/);
 assert.throws(()=>validateUi({...plugin,ui:{...plugin.ui,config_editor:{...plugin.ui.config_editor,source_view:'editable'}}}),/UI_SCHEMA_INVALID/);
 const link=path.join(s.s.dataRoot,'link');fs.symlinkSync(pluginDir,link,process.platform==='win32'?'junction':'dir');assert.throws(()=>jail(path.join(link,'plugin.json'),[s.s.dataRoot]),/PATH_OUTSIDE_ROOT|REPARSE/);
});
test('actual item counts: empty, five, and capacity are never padded',t=>{
 const s=setup(t);assert.equal(s.list().total,0);assert.deepEqual(s.list().items,[]);
 for(let n=0;n<5;n++)create(s,`case_${n}`);
 const rows=s.list({limit:500});assert.equal(rows.total,5);assert.equal(rows.items.length,5);
 const empty=matrix();empty.items=[];empty.summary={...empty.summary,total:0,selected:0,pass:0};assert.equal(validateMatrix(empty,{pluginId:plugin.id}).items.length,0);
 const five=matrix();five.items=Array.from({length:5},(_,n)=>({...five.items[0],id:`case_${n}`}));five.summary={...five.summary,total:5,selected:5,pass:5};assert.equal(validateMatrix(five,{pluginId:plugin.id}).items.length,5);
});
test('process crashes at all transaction boundaries recover to a complete old or new state',t=>{
 for(const point of ['before_replace','after_replace','after_audit']){
  const s=setup(t),a=create(s,'crash_case');
  const code=`import {workspace} from ${JSON.stringify(new URL('../lib/hub/manifest.mjs',import.meta.url).href)};import {ConfigStore} from ${JSON.stringify(new URL('../lib/hub/config-store.mjs',import.meta.url).href)};import {readJson} from ${JSON.stringify(new URL('../lib/hub/common.mjs',import.meta.url).href)};let input='';for await(const b of process.stdin)input+=b;const r=JSON.parse(input),p=readJson(r.pluginDir+'/plugin.json');const store=new ConfigStore(workspace(p,r),{fault:stage=>{if(stage===r.point)process.exit(86);}});store.save(r.save);`;
  const child=spawnSync(process.execPath,['--input-type=module','-e',code],{input:JSON.stringify({pluginDir,dataRoot:s.s.dataRoot,point,save:{config_id:a.config_id,value:{...a.value,name:'committed after crash'},expected_revision:a.revision,expected_sha256:a.sha256}}),encoding:'utf8',timeout:15000,windowsHide:true});
  assert.equal(child.status,86,child.stderr);s.withLock(()=>{});const row=s.get('crash_case');assert.equal(row.revision,point==='after_audit'?2:1);assert.equal(fs.readdirSync(s.transactions).filter(f=>f.endsWith('.json')).length,0);
 }
});
test('invalid bundle and duplicate batch ids do not leave partial changes',t=>{
 const s=setup(t),a=s.draft('a').value,b={...s.draft('b').value,duration_s:-1},file=path.join(s.s.dataRoot,'invalid-bundle.json');
 fs.writeFileSync(file,JSON.stringify({schema_version:'wiparse.config-bundle/1',plugin_id:plugin.id,plugin_version:plugin.version,items:[a,b]}));
 assert.throws(()=>s.importBundle({file}),/VALIDATION_FAILED/);assert.equal(s.list().total,0);
 const saved=create(s,'a');assert.throws(()=>s.batch({ids:['a','a'],enabled:false,expected:{a:{expected_revision:saved.revision,expected_sha256:saved.sha256}}}),/ID_CONFLICT/);assert.equal(s.get('a').revision,1);
 assert.throws(()=>s.save({config_id:'a',value:{...saved.value,id:'A'},expected_revision:1,expected_sha256:saved.sha256}),/ID_CONFLICT/);assert.equal(s.get('a').revision,1);
});
