import fs from 'node:fs';
import path from 'node:path';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {loadSchema,loadUi,defaults} from './schema.mjs';
import {readJson,jail,fail,sha,canonical} from './common.mjs';

export function projectDefinition(store) {
  const spec=store.s.plugin.ui?.project_config;
  if(!spec)return null;
  if(spec.protocol!=='wiparse.project-config/1'||!spec.schema)fail('PLUGIN.UI_SCHEMA_INVALID','Unsupported project configuration protocol');
  return {schema:loadSchema(jail(path.resolve(store.s.pluginDir,spec.schema),[store.s.pluginDir]),store.s.pluginDir),ui:loadUi(spec.ui_schema?jail(path.resolve(store.s.pluginDir,spec.ui_schema),[store.s.pluginDir]):null,store.s.pluginDir)};
}
export function projectRecord(store,value,revision) {
  const record={schema_version:'wiparse.project-config/1',project_id:store.s.project,test_id:path.basename(store.s.testDir),plugin_id:store.s.plugin.id,revision,schema_hash:store.projectDefinition.schema.fingerprint,value:structuredClone(value)};
  return {...record,sha256:sha(canonical(record))};
}
export function getProject(store,{required=false}={}) {
  const def=store.projectDefinition;if(!def)return null;
  let record,saved=fs.existsSync(store.s.projectFile);
  if(saved){
    record=readJson(jail(store.s.projectFile,store.s.roots),1048576);
    const {sha256,...body}=record;
    if(record.schema_version!=='wiparse.project-config/1'||record.project_id!==store.s.project||record.test_id!==path.basename(store.s.testDir)||record.plugin_id!==store.s.plugin.id||!Number.isSafeInteger(record.revision)||record.revision<1||sha(canonical(body))!==sha256)fail('PROJECT.CONTENT_MISMATCH');
  }else record={...projectRecord(store,defaults(def.schema.schema,def.schema.documents),0),sha256:''};
  const errors=def.schema.errors(record.value);
  if(record.schema_hash!==def.schema.fingerprint)errors.push({path:'/',message:'PROJECT.SCHEMA_CHANGED'});
  if(required&&(!saved||errors.length))fail(!saved?'PROJECT.REQUIRED':'PROJECT.INVALID','Save valid project settings before preflight',{errors});
  return {...record,saved,errors};
}
export function projectSnapshot(store){const record=getProject(store,{required:true});if(!record)return null;const {saved,errors,...snapshot}=record;return snapshot;}
export function projectMetadata(record){return record?{revision:record.revision,sha256:record.sha256,schema_hash:record.schema_hash}:null;}
export function validateProject(store,value) {
  if(!store.projectDefinition)fail('PROJECT.UNSUPPORTED');
  const schema=store.projectDefinition.schema,errors=schema.errors(value);
  const prior=store.schema;try{store.schema=schema;store.checkPathFields(value);}finally{store.schema=prior;}
  if(errors.length)fail('PROJECT.INVALID','Project settings are invalid',{errors});
  const record=projectRecord(store,value,1);
  const result=spawnSync(process.execPath,[fileURLToPath(new URL('./configure-worker.mjs',import.meta.url))],{input:JSON.stringify({pluginDir:store.s.pluginDir,entry:store.s.plugin.entry||'index.mjs',values:[],projectOnly:true,projectConfig:record}),encoding:'utf8',timeout:10000,maxBuffer:4194304,windowsHide:true});
  let data;try{data=JSON.parse(result.stdout);}catch{fail('PROJECT.INVALID','Project validation failed');}
  if(result.error||result.status!==0||data.ok!==true)fail('PROJECT.INVALID','Plugin rejected project settings',{errors:data.errors||[]});
  return data.warnings||[];
}
export function saveProject(store,params){return store.withLock(()=>{
  const old=getProject(store);if(!old)fail('PROJECT.UNSUPPORTED');
  if(store.runLocks())fail('CONFIG.LOCKED_BY_RUN');
  if(old.revision!==params.expected_revision||old.sha256!==params.expected_sha256)fail('CONFIG.REVISION_CONFLICT');
  const warnings=validateProject(store,params.value),record=projectRecord(store,params.value,old.revision+1);
  if(Buffer.byteLength(JSON.stringify(record))>1048576)fail('CONFIG.REPOSITORY_LIMIT');
  store.commit([{file:store.s.projectFile,bytes:JSON.stringify(record,null,2)+'\n'}],[{operation:'project.save',old_revision:old.revision,new_revision:record.revision,old_sha256:old.sha256,new_sha256:record.sha256}]);
  return {...record,saved:true,warnings,preflight_invalidated:true};
});}

function migrationPlan(store){
  if(!store.projectDefinition)fail('PROJECT.UNSUPPORTED');
  const rows=store.scan(),project=getProject(store);
  const child=spawnSync(process.execPath,[fileURLToPath(new URL('./configure-worker.mjs',import.meta.url))],{input:JSON.stringify({pluginDir:store.s.pluginDir,entry:store.s.plugin.entry||'index.mjs',migration:true,values:rows.map(r=>r.value),projectConfig:project?.saved?project:null}),encoding:'utf8',timeout:10000,maxBuffer:67108864,windowsHide:true});
  let plan;try{plan=JSON.parse(child.stdout);}catch{fail('PROJECT.MIGRATION_FAILED');}
  if(child.error||child.status!==0||!Array.isArray(plan.conflicts)||!Array.isArray(plan.items))fail('PROJECT.MIGRATION_FAILED');
  const basis={items:rows.map(r=>({id:r.id,sha256:r.sha256})),project_sha256:project.sha256,plan,schema_hash:store.schemaHash};
  return {...plan,plan_sha256:sha(canonical(basis))};
}
export function previewMigration(store){return store.withLock(()=>{const p=migrationPlan(store);return {...p,items:p.items.map(r=>({id:r.id,name:r.value?.name}))};});}
export function commitMigration(store,params){return store.withLock(()=>{
  if(store.runLocks())fail('CONFIG.LOCKED_BY_RUN');
  const plan=migrationPlan(store);
  if(plan.plan_sha256!==params.plan_sha256)fail('CONFIG.REVISION_CONFLICT','Migration input changed');
  if(!plan.ok||plan.conflicts.length)fail('PROJECT.MIGRATION_CONFLICT','Resolve project binding differences explicitly',{conflicts:plan.conflicts});
  if(!plan.items.length)return {...plan,migrated:0};
  const old=getProject(store),record=projectRecord(store,plan.project_value,old.revision+1);
  validateProject(store,record.value);
  const rows=store.scan();
  if(plan.items.length!==rows.length||new Set(plan.items.map(r=>r.id)).size!==rows.length||plan.items.some(r=>!rows.some(o=>o.id===r.id)||r.value?.id!==r.id))fail('PROJECT.MIGRATION_FAILED','Migration must preserve all stable IDs');
  const values=plan.items.map(item=>({...item.value,editor_revision:rows.find(r=>r.id===item.id).revision+1,updated_at:new Date().toISOString()}));
  const sizes=values.map(v=>Buffer.byteLength(JSON.stringify(v,null,2)+'\n'));
  if(values.length>store.s.maxItems||new Set(values.map(v=>v.id.toLowerCase())).size!==values.length||sizes.some(n=>n>store.s.maxItemBytes)||sizes.reduce((a,b)=>a+b,0)>store.s.maxTotalBytes||Buffer.byteLength(JSON.stringify(record,null,2)+'\n')>1048576)fail('CONFIG.REPOSITORY_LIMIT');
  store.validate(values,record);
  const backup={schema_version:'wiparse.project-migration-backup/1',project_config:old.saved?old:null,items:rows.map(r=>({id:r.id,bytes:r.bytes.toString('base64')})),plan_sha256:plan.plan_sha256};
  const entries=[{file:path.join(path.dirname(store.s.projectFile),'backups','migration-'+plan.plan_sha256+'.json'),bytes:JSON.stringify(backup,null,2)},...values.map(value=>({file:store.file(value.id),bytes:JSON.stringify(value,null,2)+'\n'})),{file:store.s.projectFile,bytes:JSON.stringify(record,null,2)+'\n'}];
  store.commit(entries,[{operation:'project.migration',plan_sha256:plan.plan_sha256,project_sha256:record.sha256,config_ids:values.map(v=>v.id)}]);
  return {migrated:values.length,project_config:projectMetadata(record),preflight_invalidated:true};
});}
