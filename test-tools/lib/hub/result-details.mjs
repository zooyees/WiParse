import path from 'node:path';
import fs from 'node:fs';
import {readMatrix} from './matrix.mjs';
import {readJson,jail,fail} from './common.mjs';
export function resultDetails(store,params){
  const matrix=readMatrix(store.s,{limit:500}),row=matrix.items?.find(r=>r.id===params.config_id);
  if(!row||matrix.run_id!==params.run_id||row.attempt!==params.attempt)fail('RESULT.STALE_SELECTION');
  if(!row.result_ref)return {config_id:row.id,run_id:matrix.run_id,attempt:row.attempt,available:false};
  const root=path.join(store.s.dataRoot,'projects',store.s.project),file=jail(path.resolve(root,row.result_ref),[store.s.testDir]);
  if(!fs.existsSync(file))return {config_id:row.id,run_id:matrix.run_id,attempt:row.attempt,available:false,error:'RESULT.MISSING_EVIDENCE'};
  // Result JSON only; never read an unbounded log or follow an external artifact.
  if(path.extname(file).toLowerCase()!=='.json')fail('RESULT.INVALID_REFERENCE');
  const value=readJson(file,524288);
  const contract=store.s.plugin.ui?.result_details;
  const required=['schema_version','scope','item_id','run_id','attempt','config_sha256'];
  if(contract?.protocol!=='wiparse.result-details/1'||required.some(k=>value[k]==null)||matrix.project_config&&!value.project_config||row.attempt_id&&!value.attempt_id)return {config_id:row.id,run_id:matrix.run_id,attempt:row.attempt,available:false,result_ref:row.result_ref,error:'RESULT.UNVERIFIABLE_HISTORY'};
  if(value.schema_version!==contract.schema_version||value.scope.project_id!==store.s.project||value.scope.test_id!==path.basename(store.s.testDir)||value.scope.plugin_id!==store.s.plugin.id||value.item_id!==row.id||value.run_id!==matrix.run_id||value.attempt!==row.attempt||row.attempt_id&&value.attempt_id!==row.attempt_id||value.config_sha256!==row.config_sha256||matrix.project_config&&['revision','sha256','schema_hash'].some(k=>matrix.project_config[k]!==value.project_config[k]))fail('RESULT.CONTENT_MISMATCH');
  const evidence=[];
  for(const [label,entry] of Object.entries(value.evidence?.artifacts||{}).slice(0,64)){
    if(typeof entry?.path!=='string')continue;
    try{const f=jail(path.resolve(path.dirname(file),entry.path),[path.dirname(file)]);evidence.push({label,reference:path.relative(root,f).split(path.sep).join('/'),available:fs.existsSync(f)});}catch{evidence.push({label,available:false,error:'RESULT.INVALID_REFERENCE'});}
  }
  const log=path.join(path.dirname(file),'logs','serial.log');if(fs.existsSync(jail(log,[path.dirname(file)])))evidence.push({label:'serial.log',reference:path.relative(root,log).split(path.sep).join('/'),available:true});
  return {config_id:row.id,run_id:matrix.run_id,attempt:row.attempt,available:true,result_ref:row.result_ref,evidence,value};
}
