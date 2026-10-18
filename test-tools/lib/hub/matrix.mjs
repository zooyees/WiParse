import fs from 'node:fs';
import path from 'node:path';
import {jail,fail,id,readJson,page,sha,canonical,atomicWrite} from './common.mjs';
export const STATES=Object.freeze(['pending','running','pass','fail','error','skipped','aborted','inconclusive']);
const terminal = status=>!['pending','running'].includes(status);
export function validateMatrix(value,{previous=null,pluginId,projectRoot,maxItems=500}={}) {
  if (!value || value.schema_version!=='wiparse.test-matrix/1' || value.plugin_id!==pluginId || !Number.isSafeInteger(value.revision) || value.revision<0 || typeof value.run_id!=='string' || !Array.isArray(value.items)) fail('MATRIX.INVALID_SNAPSHOT','Invalid matrix header');
  if(value.run_id.length>128||typeof value.updated_at!=='string'||!Number.isFinite(Date.parse(value.updated_at))||typeof value.preflight_revision!=='string'||value.preflight_revision.length>128)fail('MATRIX.INVALID_SNAPSHOT','Invalid run identity or timestamp');
  if(Object.keys(value).some(k=>!['project_config','schema_version','revision','plugin_id','run_id','preflight_revision','updated_at','active_item_id','summary','items'].includes(k)))fail('MATRIX.INVALID_SNAPSHOT','Unsupported snapshot fields');
  if (value.items.length>Math.min(500,maxItems)) fail('MATRIX.TOO_LARGE');
  if(value.project_config!=null&&(!Number.isSafeInteger(value.project_config.revision)||value.project_config.revision<1||! /^[a-f0-9]{64}$/i.test(value.project_config.sha256||'')||! /^[a-f0-9]{64}$/i.test(value.project_config.schema_hash||'')))fail('MATRIX.INVALID_SNAPSHOT','Invalid project binding');
  const seen=new Set(), counts=Object.fromEntries(STATES.map(s=>[s,0])); let selected=0;
  const old=previous?.run_id===value.run_id?previous:null;
  if (old && (value.revision<old.revision || (value.revision===old.revision&&canonical(value)!==canonical(old)))) fail('MATRIX.REVISION_ROLLBACK');
  const prior=new Map((old?.items || []).map(row=>[row.id,row]));
  const allowed=new Set('attempt_id id name order tags selected status phase progress elapsed_s config_revision config_sha256 attempt loop_index message result_ref error_code estimated_duration_s'.split(' '));
  const items=value.items.map(row=>{
    if(row.attempt_id!=null&&(typeof row.attempt_id!=='string'||!/^[A-Za-z0-9._-]{1,128}$/.test(row.attempt_id)))fail('MATRIX.INVALID_SNAPSHOT','Invalid attempt identity');
    id(row.id); if (seen.has(row.id)) fail('MATRIX.INVALID_SNAPSHOT','Duplicate row id'); seen.add(row.id);
    if (typeof row.name!=='string' || [...row.name].length>128 || typeof row.selected!=='boolean' || !STATES.includes(row.status) || !Number.isInteger(row.attempt) || row.attempt<1) fail('MATRIX.INVALID_SNAPSHOT','Invalid row');
    if(typeof row.phase!=='string'||row.phase.length>128||Object.keys(row).some(k=>!allowed.has(k)))fail('MATRIX.INVALID_SNAPSHOT','Invalid or unsupported row fields');
    for(const key of ['order','config_revision','loop_index'])if(row[key]!=null&&(!Number.isSafeInteger(row[key])||row[key]<0))fail('MATRIX.INVALID_SNAPSHOT','Invalid row index or revision');
    if(row.config_sha256!=null&&(typeof row.config_sha256!=='string'||! /^[a-f0-9]{64}$/i.test(row.config_sha256)))fail('MATRIX.INVALID_SNAPSHOT','Invalid configuration hash');
    if(row.error_code!=null&&(typeof row.error_code!=='string'||row.error_code.length>128))fail('MATRIX.INVALID_SNAPSHOT','Invalid error code');
    if (!Array.isArray(row.tags) || row.tags.length>32 || row.tags.some(t=>typeof t!=='string'||[...t].length>64) || typeof row.message!=='string' || [...row.message].length>256) fail('MATRIX.INVALID_SNAPSHOT','Invalid row text');
    if (row.progress!==null && (typeof row.progress!=='number'||!Number.isFinite(row.progress)||row.progress<0||row.progress>1)) fail('MATRIX.INVALID_SNAPSHOT','Invalid progress');
    for (const field of ['elapsed_s','estimated_duration_s']) if (row[field]!=null && (typeof row[field]!=='number'||!Number.isFinite(row[field])||row[field]<0)) fail('MATRIX.INVALID_SNAPSHOT','Invalid elapsed time');
    if (row.result_ref!=null) {
      if (typeof row.result_ref!=='string'||row.result_ref.length>1024||path.isAbsolute(row.result_ref)||/^[A-Za-z]:|^[\\/]|^[a-z]+:/i.test(row.result_ref)||row.result_ref.split(/[\\/]/).includes('..')) fail('CONFIG.PATH_OUTSIDE_ROOT','Invalid project result reference');
      if (projectRoot) jail(path.join(projectRoot,row.result_ref),[projectRoot]);
    }
    const before=prior.get(row.id);
    if (before && (row.attempt<before.attempt || (row.attempt===before.attempt&&terminal(before.status)&&row.status!==before.status))) fail('MATRIX.INVALID_SNAPSHOT','Terminal status requires a new attempt');
    if(before&&row.attempt>before.attempt&&!terminal(row.status)&&row.result_ref)fail('MATRIX.INVALID_SNAPSHOT','New attempt cannot point at a prior result');
    counts[row.status]++; if (row.selected) selected++;
    const output=Object.fromEntries(Object.entries(row).filter(([key,val])=>allowed.has(key)&&(!val||typeof val!=='object'||key==='tags')));
    return output;
  });
  if (counts.running>1) fail('MATRIX.INVALID_SNAPSHOT','Only one running item is supported');
  const summary={total:items.length,selected,...counts};
  for (const [key,count] of Object.entries(summary)) if (value.summary?.[key]!==count) fail('MATRIX.INVALID_SNAPSHOT','Summary counts differ from rows');
  if (value.active_item_id && !items.some(row=>row.id===value.active_item_id&&row.status==='running')) fail('MATRIX.INVALID_SNAPSHOT','Invalid active item');
  return {...value,items,summary};
}
export function readMatrix(space,params={}) {
  const cacheFile=path.join(space.editorRoot,'matrix-cache.json'); let cache=null;
  try { cache=readJson(jail(cacheFile,space.roots),5*1048576); } catch {}
  if (!fs.existsSync(space.matrix)) return {available:false,summary:null,items:[],total:0};
  const stat=fs.statSync(jail(space.matrix,[space.testDir]));
  const signature=`${stat.mtimeMs}:${stat.size}`;
  let snapshot=cache?.snapshot, error=cache?.error || null;
  if (cache?.signature!==signature || params.refresh) {
    try {
      if(stat.size>4*1048576) fail('MATRIX.TOO_LARGE');
      const candidate=readJson(space.matrix,4*1048576);
      snapshot=validateMatrix(candidate,{previous:snapshot,pluginId:space.plugin.id,projectRoot:path.join(space.dataRoot,'projects',space.project),maxItems:space.maxItems});
      error=null;
      atomicWrite(cacheFile,JSON.stringify({signature,snapshot}));
    } catch(e) { error=e.code || 'MATRIX.INVALID_SNAPSHOT'; atomicWrite(cacheFile,JSON.stringify({signature,snapshot,error})); }
  }
  if (!snapshot) return {available:false,error:error||'MATRIX.INVALID_SNAPSHOT',items:[],total:0};
  return {available:true,error,...(snapshot.project_config?{project_config:snapshot.project_config}:{}),revision:snapshot.revision,run_id:snapshot.run_id,preflight_revision:snapshot.preflight_revision,active_item_id:snapshot.active_item_id,updated_at:snapshot.updated_at,summary:snapshot.summary,...page(snapshot.items,params)};
}
