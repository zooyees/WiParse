import {resultDetails} from './lib/hub/result-details.mjs';
import {getProject,saveProject,validateProject,previewMigration,commitMigration} from './lib/hub/project-config.mjs';
/** Bounded IPC for the generic Hub workspace. Never run on the GUI thread. */
import fs from 'node:fs';
import {inspectFile} from './lib/hub/file-inspect.mjs';
import {controlState,requestControl} from './lib/hub/run-control.mjs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {workspace} from './lib/hub/manifest.mjs';
import {ConfigStore} from './lib/hub/config-store.mjs';
import {readMatrix} from './lib/hub/matrix.mjs';
import {preflightState} from './lib/hub/run-binding.mjs';
import {readJson,redact,fail} from './lib/hub/common.mjs';
import {redactSchema} from './lib/hub/schema.mjs';

export function execute(request) {
  const {context,method,params={}}=request;
  if(params._invalid_json)fail('CONFIG.INVALID_REQUEST');
  if(!request.internal&&(method.endsWith('config.save')||method.endsWith('project.save'))){const inspect=value=>{if(value==='[REDACTED]'||value==='[LOCAL_PATH]')fail('CONFIG.REDACTED_VALUE','Provide actual configuration values, not a redacted read response');if(value&&typeof value==='object')for(const v of Object.values(value))inspect(v);};inspect(params.value);}
  const plugin=readJson(path.join(context.pluginDir,'plugin.json'));
  const store=new ConfigStore(workspace(plugin,context));
  store.ui={...store.ui,source_view:plugin.ui.config_editor.source_view||store.ui.source_view||'read_only',execution_controls:plugin.ui.execution_controls??null};
  let data;
  const operation=method.replace(/^ui\.test_tool\./,'');
  if(operation==='config.inspect_file'&&!request.internal)fail('CONFIG.INTERNAL_METHOD');
  const perform=()=>{switch(operation) {
    case 'workspace.open': data={...store.list({limit:500}),schema:store.schema.schema,documents:store.schema.documents,ui:store.ui,archives:store.archives(),preflight:preflightState(store),locked:store.runLocks(),project_config:projectData(store),matrix:matrixData(store,{limit:500}),...(request.internal?{matrix_file:store.s.matrix}:{})};break;
    case 'project.get':data=projectData(store);break;
    case 'project.save':data=saveProject(store,params);break;
    case 'project.validate':data={valid:true,warnings:validateProject(store,params.value)};break;
    case 'project.migration.preview':data=previewMigration(store);break;
    case 'project.migration.commit':data=commitMigration(store,params);break;
    case 'config.list': data=store.list(params);break;
    case 'config.get': {const {bytes,...row}=store.get(params.config_id);data={...row,config_id:row.id,schema:store.schema.schema,documents:store.schema.documents,ui:store.ui,schema_fingerprint:store.schemaHash};break;}
    case 'config.inspect_file': data=inspectFile(store,params);break;
    case 'config.create': data=store.draft(params.config_id);break;
    case 'config.duplicate': data=store.draft(params.new_id,store.get(params.config_id).value);break;
    case 'config.save': data=store.save(params,params.operation==='duplicate'?'duplicate':'save');break;
    case 'config.validate': {const errors=store.schema.errors(params.value);try{store.checkPathFields(params.value);}catch(e){errors.push({path:'/',message:e.code});}data={valid:!errors.length,errors};break;}
    case 'config.archive': data=store.archive(params);break;
    case 'config.restore': data=store.restore(params);break;
    case 'config.archives': data={items:store.archives()};break;
    case 'config.batch': data=store.batch(params);break;
    case 'config.import': data=store.importBundle(params);break;
    case 'config.export': data=store.exportBundle(params);break;
    case 'result.get':data=resultDetails(store,params);break;
    case 'matrix.get': data=matrixData(store,params);break;
    case 'run.control':data=requestControl(store,params);break;
    case 'preflight.state': data=preflightState(store);break;
    default: fail('CONFIG.UNKNOWN_METHOD');
  }return data;};
  // A read holds the same mutex through its entire snapshot, including recovery.
  // Mutations own their transactions; matrix polling stays independent of imports.
  data=['project.save','project.migration.preview','project.migration.commit','config.save','config.archive','config.restore','config.batch','config.import','config.export','matrix.get','result.get','config.validate','run.control'].includes(operation)?perform():store.withLock(perform);
  if(!request.internal){if(data?.value){const def=operation.startsWith('project.')?store.projectDefinition?.schema:store.schema;if(def)data.value=redactSchema(data.value,def.schema,def.documents);}if(data?.project_config?.value&&store.projectDefinition)data.project_config.value=redactSchema(data.project_config.value,store.projectDefinition.schema.schema,store.projectDefinition.schema.documents);const errors=data?.errors;data=redact(data);if(errors)data.errors=safeErrors(errors);}
  return data;
}
function safeErrors(errors){return (Array.isArray(errors)?errors:[]).slice(0,100).map(e=>({...redact(e),path:typeof e.path==='string'&&/^\/(?:[\w~.-]+\/?)*$/.test(e.path)?e.path:'/'}));}
function matrixData(store,params){
  const data=readMatrix(store.s,params),lock=store.runLocks();
  data.control=controlState(store);
  if(data.available&&lock&&data.run_id!==lock.run_id)data.error='MATRIX.STALE_RUN';
  else if(data.available&&!lock&&data.summary.running>0)data.error='MATRIX.INTERRUPTED_RUN';
  if(lock&&lock.schema_hash!==store.schemaHash)data.workspace_locked=true;
  return data;
}
if(process.argv[1]&&path.resolve(process.argv[1])===fileURLToPath(import.meta.url)) {
  try {
    let input='';for await(const part of process.stdin){input+=part;if(Buffer.byteLength(input)>2*1048576) fail('CONFIG.REQUEST_TOO_LARGE');}
    const request=JSON.parse(input),data=execute(request);
    const output=JSON.stringify({ok:true,data});if(Buffer.byteLength(output)>6*1048576) fail('CONFIG.RESPONSE_TOO_LARGE');
    process.stdout.write(output);
  } catch(e) {const details=redact(e.data||{});if(e.data?.errors)details.errors=safeErrors(e.data.errors);process.stdout.write(JSON.stringify({ok:false,error:{code:e.code||'CONFIG.OPERATION_FAILED',message:e.code||'Configuration operation failed',...details}}));process.exitCode=1;}
}

function projectData(store){const record=getProject(store);return record?{...record,schema:store.projectDefinition.schema.schema,documents:store.projectDefinition.schema.documents,ui:store.projectDefinition.ui}:null;}
