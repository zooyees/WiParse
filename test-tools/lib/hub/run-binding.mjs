import {projectSnapshot,projectMetadata,validateProject} from './project-config.mjs';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {sha,canonical,id,fail,jail,readJson,atomicWrite,appendSync} from './common.mjs';

export function manifest(store,selectedIds) {
  const rows=store.scan();
  const requested=Array.isArray(selectedIds)?selectedIds:rows.filter(r=>r.enabled).map(r=>r.id);
  if (!requested.length || new Set(requested).size!==requested.length || requested.some(key=>!rows.some(r=>r.id===key&&r.enabled))) fail('PREFLIGHT.REQUIRED','Select at least one enabled configuration');
  if(rows.some(r=>requested.includes(r.id)&&r.errors.length)) fail('SCHEMA.VALIDATION_FAILED','Selected configurations are invalid');
  for(const row of rows.filter(r=>requested.includes(r.id)))store.checkPathFields(row.value);
  const items=rows.map(row=>({id:row.id,revision:row.revision,sha256:row.sha256})).sort((a,b)=>a.id.localeCompare(b.id));
  const selected=[...requested].sort();
  const shared=projectSnapshot(store);
  const document={...(shared?{project_config:projectMetadata(shared)}:{}),plugin_id:store.s.plugin.id,plugin_version:store.s.plugin.version,schema_hash:store.schemaHash,run_params_sha256:sha(canonical(store.s.params)),items,selected_item_ids:selected};
  return {...document,manifest_sha256:sha(canonical(document)),values:rows.map(row=>row.value),...(shared?{project_config_snapshot:shared}:{})};
}
export function assertStationIdle(store){
  const file=jail(path.join(store.s.dataRoot,'.station-run.lock'),[store.s.dataRoot]);
  if(!fs.existsSync(file))return;
  const lock=readJson(file);if(!Number.isInteger(lock.pid)||lock.pid<=0)fail('RUN.STATION_BUSY');
  try{process.kill(lock.pid,0);fail('RUN.STATION_BUSY');}catch(e){if(e.code!=='ESRCH')throw e;}
}
export function preflightState(store) {
  const file=path.join(store.s.editorRoot,'preflight.json'); if (!fs.existsSync(file)) return {valid:false,error:'PREFLIGHT.REQUIRED'};
  const prior=readJson(jail(file,store.s.roots));
  try { const current=manifest(store,prior.selected_item_ids); return {valid:current.manifest_sha256===prior.manifest_sha256,revision:prior.revision,manifest_sha256:prior.manifest_sha256,selected_item_ids:prior.selected_item_ids,error:current.manifest_sha256===prior.manifest_sha256?null:'PREFLIGHT.STALE'}; }
  catch { return {valid:false,error:'PREFLIGHT.STALE'}; }
}
export function completePreflight(store,frozen,result) {
  return store.withLock(()=>{
    if (result.ok!==true || !result.test_matrix || result.test_matrix.manifest_sha256!==frozen.manifest_sha256 || result.test_matrix.revision!==frozen.manifest_sha256 || (result.test_matrix.item_count!==undefined&&result.test_matrix.item_count!==frozen.selected_item_ids.length)) fail('PREFLIGHT.REQUIRED','Plugin did not confirm the frozen manifest');
    const current=manifest(store,frozen.selected_item_ids); if (current.manifest_sha256!==frozen.manifest_sha256) fail('PREFLIGHT.STALE');
    const record={manifest_sha256:frozen.manifest_sha256,revision:frozen.manifest_sha256,selected_item_ids:frozen.selected_item_ids,at:new Date().toISOString()};
    atomicWrite(path.join(store.s.editorRoot,'preflight.json'),JSON.stringify(record)); return record;
  });
}
export function beginRun(store,selectedIds,runDir,expected=null) {
  return store.withLock(()=>{
    if (store.runLocks()) fail('RUN.STATION_BUSY');
    const state=preflightState(store); if (!state.valid) fail(state.error);
    const frozen=manifest(store,selectedIds); if (frozen.manifest_sha256!==state.manifest_sha256) fail('PREFLIGHT.STALE','Selection changed since preflight');
    if(expected&&(expected.config_manifest_sha256!==frozen.manifest_sha256||expected.preflight_revision!==state.revision))fail('PREFLIGHT.STALE','Overlay does not bind the current preflight');
    jail(runDir,[store.s.testDir]);
    const binding={run_id:crypto.randomUUID(),pid:process.pid,ids:frozen.selected_item_ids,schema_hash:store.schemaHash,...(frozen.project_config?{project_config:frozen.project_config}:{}),manifest_sha256:frozen.manifest_sha256,started_at:new Date().toISOString()};
    const stationLock=path.join(store.s.dataRoot,'.station-run.lock');
    fs.mkdirSync(path.dirname(stationLock),{recursive:true});
    if (fs.existsSync(stationLock)) {
      let prior;try { prior=readJson(jail(stationLock,[store.s.dataRoot])); } catch { fail('RUN.STATION_BUSY','Unknown station lock'); }
      try { process.kill(prior.pid,0); fail('RUN.STATION_BUSY'); } catch(e) { if(e.code!=='ESRCH') throw e; }
      fs.unlinkSync(stationLock); appendSync(store.s.audit,JSON.stringify({at:new Date().toISOString(),operation:'recover_station_lock',reason:'owner_process_exited',actor:'local-user'})+'\n');
    }
    const fd=fs.openSync(stationLock,'wx',0o600); try {fs.writeFileSync(fd,JSON.stringify(binding));fs.fsyncSync(fd);} finally {fs.closeSync(fd);}
    try {
      fs.mkdirSync(runDir,{recursive:true});
      const snapshotDir=path.join(runDir,'config-snapshot');fs.mkdirSync(snapshotDir,{recursive:true});
      for(const value of frozen.values) atomicWrite(path.join(snapshotDir,id(value.id)+'.json'),JSON.stringify(value,null,2)+'\n');
      atomicWrite(path.join(runDir,'config-manifest.json'),JSON.stringify({...frozen,values:undefined,project_config_snapshot:undefined},null,2));
      if(frozen.project_config_snapshot)atomicWrite(path.join(runDir,'project-config.json'),JSON.stringify(frozen.project_config_snapshot,null,2)+'\n');
      atomicWrite(path.join(store.s.editorRoot,'run-binding.json'),JSON.stringify(binding));
      return {...binding,...frozen,preflight_revision:state.revision,snapshotDir,stationLock};
    } catch(e) {fs.unlinkSync(stationLock);throw e;}
  });
}
export function endRun(store,binding) {
  const files=[path.join(store.s.editorRoot,'run-binding.json'),binding.stationLock];
  for(const file of files) if(fs.existsSync(file)&&readJson(jail(file,store.s.roots)).run_id===binding.run_id) fs.unlinkSync(file);
}
