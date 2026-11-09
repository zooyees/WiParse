import {restoreVisibleDocument} from './lib/hub/visible-document.mjs';
import {resultDetails} from './lib/hub/result-details.mjs';
import {getProject,saveProject,validateProject,previewMigration,commitMigration} from './lib/hub/project-config.mjs';
import {activateProjectFile,exportProjectFile,importProjectFile,projectFileSnapshot} from './lib/hub/project-files.mjs';
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
    case 'workspace.open': data={...store.list({limit:500}),schema:store.schema.schema,documents:store.schema.documents,ui:store.ui,archives:store.archives(),preflight:preflightState(store),locked:store.runLocks(),project_config:projectData(store),project_files:projectFileSnapshot(store),matrix:matrixData(store,{limit:500}),...(request.internal?{matrix_file:store.s.matrix}:{})};break;
    case 'project.file.activate': data=activateProjectFile(store,params);break;
    case 'project.file.import': data=importProjectFile(store,params,{internal:request.internal===true});break;
    case 'project.file.export': data=exportProjectFile(store,params,{internal:request.internal===true});break;
    case 'project.get':data=projectData(store);break;
    case 'project.save':data=saveProject(store,params);break;
    case 'project.edit': {
      if(context.activePlugin!==plugin.id||params.expected_project!==store.s.project||params.schema_fingerprint!==store.projectDefinition?.schema.fingerprint)fail('CONFIG.CONTEXT_CHANGED');
      const current=store.withLock(()=>getProject(store));if(!current)fail('PROJECT.UNSUPPORTED');
      if(current.revision!==params.expected_revision||current.sha256!==params.expected_sha256)fail('CONFIG.REVISION_CONFLICT');
      const value=restoreVisibleDocument(params.value,current.value);
      if(params.validate_only){data={valid:true,warnings:validateProject(store,value)};break;}
      data=saveProject(store,{...params,value});break;
    }
    case 'project.patch_dut':if(!request.internal&&context.activePlugin!==plugin.id)fail('CONFIG.CONTEXT_CHANGED');data=patchProjectDut(store,params);break;
    case 'project.patch_resource':if(!request.internal&&context.activePlugin!==plugin.id)fail('CONFIG.CONTEXT_CHANGED');data=patchProjectResource(store,params);break;
    case 'project.patch_serial':if(!request.internal&&context.activePlugin!==plugin.id)fail('CONFIG.CONTEXT_CHANGED');data=patchProjectSerial(store,params);break;
    case 'project.patch_dongle_uart':if(!request.internal&&context.activePlugin!==plugin.id)fail('CONFIG.CONTEXT_CHANGED');data=patchProjectDongleUart(store,params);break;
    case 'project.patch_wireless_station':if(!request.internal&&context.activePlugin!==plugin.id)fail('CONFIG.CONTEXT_CHANGED');data=patchWirelessStation(store,params);break;
    case 'project.validate':data={valid:true,warnings:validateProject(store,params.value)};break;
    case 'project.migration.preview':data=previewMigration(store);break;
    case 'project.migration.commit':data=commitMigration(store,params);break;
    case 'config.list': data=store.list(params);break;
    case 'config.get': {const {bytes,...row}=store.get(params.config_id);data={...row,config_id:row.id,schema:store.schema.schema,documents:store.schema.documents,ui:store.ui,schema_fingerprint:store.schemaHash};break;}
    case 'config.inspect_file': data=inspectFile(store,params);break;
    case 'config.create': data=store.draft(params.config_id);break;
    case 'config.duplicate': data=store.draft(params.new_id,store.get(params.config_id).value);break;
    case 'config.save': data=store.save(params,params.operation==='duplicate'?'duplicate':'save');break;
    case 'config.edit': {
      if(context.activePlugin!==plugin.id||params.expected_project!==store.s.project||params.schema_fingerprint!==store.schemaHash)fail('CONFIG.CONTEXT_CHANGED');
      const current=store.withLock(()=>{const row=store.get(params.config_id);store.expected(row,params);return row;});
      if(params.value?.id!==current.id)fail('CONFIG.INVALID_REQUEST','Use duplicate to create another identifier');
      const value=restoreVisibleDocument(params.value,current.value);
      data=store.save({...params,value},'save');break;
    }
    case 'config.patch': if(!request.internal&&context.activePlugin!==plugin.id)fail('CONFIG.CONTEXT_CHANGED');data=patchItemProperties(store,params);break;
    case 'config.clone': if(!request.internal&&context.activePlugin!==plugin.id)fail('CONFIG.CONTEXT_CHANGED');data=cloneItem(store,params);break;
    case 'config.create_saved': if(!request.internal&&context.activePlugin!==plugin.id)fail('CONFIG.CONTEXT_CHANGED');data=createItem(store,params);break;
    case 'config.select_firmware': if(!request.internal&&context.activePlugin!==plugin.id)fail('CONFIG.CONTEXT_CHANGED');data=selectFirmware(store,params);break;
    case 'config.validate': {const errors=store.blockingErrors(params.value);try{store.checkPathFields(params.value);}catch(e){errors.push({path:'/',message:e.code});}data={valid:!errors.length,errors};break;}
    case 'config.archive': data=store.archive(params);break;
    case 'config.restore': data=store.restore(params);break;
    case 'config.archives': data={items:store.archives()};break;
    case 'config.batch': data=store.batch(params);break;
    case 'config.import': data=store.importBundle(params);break;
    case 'config.export': data=store.exportBundle(params);break;
    case 'result.get':data=resultDetails(store,params);break;
    case 'matrix.get': data={...matrixData(store,params),control:controlState(store)};break;
    case 'run.control':data=requestControl(store,params);break;
    case 'preflight.state': data=preflightState(store);break;
    default: fail('CONFIG.UNKNOWN_METHOD');
  }return data;};
  // A read holds the same mutex through its entire snapshot, including recovery.
  // Mutations own their transactions; matrix polling stays independent of imports.
  data=['project.save','project.edit','project.patch_dut','project.patch_resource','project.patch_serial','project.patch_dongle_uart','project.patch_wireless_station','project.migration.preview','project.migration.commit','project.file.activate','project.file.import','project.file.export','config.save','config.edit','config.patch','config.clone','config.create_saved','config.select_firmware','config.archive','config.restore','config.batch','config.import','config.export','matrix.get','result.get','config.validate','run.control'].includes(operation)?perform():store.withLock(perform);
  if(!request.internal){if(data?.value){const def=operation.startsWith('project.')?store.projectDefinition?.schema:store.schema;if(def)data.value=redactSchema(data.value,def.schema,def.documents);}if(data?.project_config?.value&&store.projectDefinition)data.project_config.value=redactSchema(data.project_config.value,store.projectDefinition.schema.schema,store.projectDefinition.schema.documents);const errors=data?.errors;data=redact(data);if(errors)data.errors=safeErrors(errors);}
  return data;
}
function cloneItem(store,params){
  if(store.s.plugin.id!=='fpga-firmware-test'||typeof params.expected_project!=='string'||params.expected_project!==store.s.project)fail('CONFIG.CONTEXT_CHANGED');
  if(typeof params.name!=='string'||!params.name.trim()||params.name.length>128||/[\x00-\x1f\x7f]/.test(params.name))fail('CONFIG.INVALID_REQUEST');
  const source=store.withLock(()=>{const row=store.get(params.source_id);store.expected(row,{expected_revision:params.source_revision,expected_sha256:params.source_sha256});return row;});
  const draft=store.withLock(()=>store.draft(params.new_id,source.value));draft.value.name=params.name.trim();
  return store.save({config_id:draft.config_id,value:draft.value,expected_revision:0,expected_sha256:'',source_snapshot:{config_id:source.id,revision:source.revision,sha256:source.sha256}},'duplicate');
}
function createItem(store,params){
  if(store.s.plugin.id!=='fpga-firmware-test'||typeof params.expected_project!=='string'||params.expected_project!==store.s.project)fail('CONFIG.CONTEXT_CHANGED');
  if(typeof params.name!=='string'||!params.name.trim()||params.name.length>128||/[\x00-\x1f\x7f]/.test(params.name))fail('CONFIG.INVALID_REQUEST');
  const draft=store.withLock(()=>store.draft(params.config_id));draft.value.name=params.name.trim();draft.value.enabled=false;
  return store.save({config_id:draft.config_id,value:draft.value,expected_revision:0,expected_sha256:''},'create');
}
function selectFirmware(store,params){
  if(store.s.plugin.id!=='fpga-firmware-test'||typeof params.expected_project!=='string'||params.expected_project!==store.s.project)fail('CONFIG.CONTEXT_CHANGED');
  if(typeof params.path!=='string'||!params.path||params.path.length>4096||!['.hex','.bin'].includes(path.extname(params.path).toLowerCase()))fail('CONFIG.INVALID_REQUEST');
  const row=store.withLock(()=>{const item=store.get(params.config_id);store.expected(item,params);return item;});
  const info=inspectFile(store,{config_id:row.id,field:'firmware.path',path:params.path});
  const value=structuredClone(row.value);if(!value.firmware||typeof value.firmware!=='object'||Array.isArray(value.firmware))fail('CONFIG.INVALID_REQUEST');
  value.firmware={...value.firmware,path:info.path,sha256:info.sha256,format:path.extname(params.path).slice(1).toLowerCase()};
  return store.save({...params,value},'select_firmware');
}
function patchProjectDut(store,params){
  if(store.s.plugin.id!=='fpga-firmware-test'||typeof params.expected_project!=='string'||params.expected_project!==store.s.project)fail('CONFIG.CONTEXT_CHANGED');
  const patch=params.dut,fields=['model','board_revision','fpga_model','fixture_id','station_id','serial_required'];
  if(!patch||typeof patch!=='object'||Array.isArray(patch)||Object.keys(patch).length!==fields.length||Object.keys(patch).some(key=>!fields.includes(key)))fail('CONFIG.INVALID_REQUEST');
  for(const key of fields.filter(key=>key!=='serial_required'))if(typeof patch[key]!=='string'||patch[key].length>256||/[\x00-\x1f\x7f]/.test(patch[key])||['[REDACTED]','[LOCAL_PATH]'].includes(patch[key]))fail('CONFIG.INVALID_REQUEST');
  if(typeof patch.serial_required!=='boolean')fail('CONFIG.INVALID_REQUEST');
  const current=store.withLock(()=>getProject(store));if(!current?.saved)fail('PROJECT.REQUIRED');
  if(current.revision!==params.expected_revision||current.sha256!==params.expected_sha256)fail('CONFIG.REVISION_CONFLICT');
  const value=structuredClone(current.value);if(!value.dut||typeof value.dut!=='object'||Array.isArray(value.dut))fail('PROJECT.INVALID');
  value.dut={...value.dut,...patch};return saveProject(store,{expected_revision:current.revision,expected_sha256:current.sha256,value});
}
function patchProjectResource(store,params){
  if(store.s.plugin.id!=='fpga-firmware-test'||typeof params.expected_project!=='string'||params.expected_project!==store.s.project)fail('CONFIG.CONTEXT_CHANGED');
  if(typeof params.role!=='string'||!/^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/.test(params.role)||typeof params.resource!=='string'||!params.resource.trim()||params.resource.length>1024||/[\x00-\x1f\x7f]/.test(params.resource)||['[REDACTED]','[LOCAL_PATH]'].includes(params.resource))fail('CONFIG.INVALID_REQUEST');
  const current=store.withLock(()=>getProject(store));if(!current?.saved)fail('PROJECT.REQUIRED');
  if(current.revision!==params.expected_revision||current.sha256!==params.expected_sha256)fail('CONFIG.REVISION_CONFLICT');
  const value=structuredClone(current.value),binding=value.instruments?.[params.role];
  if(!binding||!['oscilloscope','dc_source','dmm','load'].includes(binding.kind)||!binding.selector||typeof binding.selector!=='object'||Array.isArray(binding.selector))fail('CONFIG.INVALID_REQUEST');
  binding.selector.resource=params.resource.trim();return saveProject(store,{expected_revision:current.revision,expected_sha256:current.sha256,value});
}
function patchProjectSerial(store,params){
  if(store.s.plugin.id!=='fpga-firmware-test'||typeof params.expected_project!=='string'||params.expected_project!==store.s.project)fail('CONFIG.CONTEXT_CHANGED');
  if(typeof params.role!=='string'||!/^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/.test(params.role)||typeof params.port!=='string'||!/^COM[1-9][0-9]{0,5}$/i.test(params.port)||!Number.isInteger(params.baud)||params.baud<1||params.baud>12000000)fail('CONFIG.INVALID_REQUEST');
  const current=store.withLock(()=>getProject(store));if(!current?.saved)fail('PROJECT.REQUIRED');
  if(current.revision!==params.expected_revision||current.sha256!==params.expected_sha256)fail('CONFIG.REVISION_CONFLICT');
  const value=structuredClone(current.value),binding=value.instruments?.[params.role];
  if(!binding||binding.kind!=='serial_port'||!binding.selector||typeof binding.selector!=='object'||Array.isArray(binding.selector))fail('CONFIG.INVALID_REQUEST');
  binding.selector.port=params.port.toUpperCase();binding.selector.baud=params.baud;
  return saveProject(store,{expected_revision:current.revision,expected_sha256:current.sha256,value});
}
function patchWirelessStation(store,params){
  if(store.s.plugin.id!=='wireless-whitebox'||params.expected_project!==store.s.project)fail('CONFIG.CONTEXT_CHANGED');
  const patch=params.patch,strings=['station_id','rx_i2c_interface','log_source','rx_start_marker','rx_identification_marker','rx_running_marker','scope_resource','power_resource','load_resource'],numbers=['log_baud','rx_i2c_clock_hz','rx_ready_timeout_s'],booleans=['scope_capture_enabled','scope_source_enabled'];
  if(!patch||typeof patch!=='object'||Array.isArray(patch)||!Object.keys(patch).length||Object.keys(patch).some(k=>![...strings,...numbers,...booleans].includes(k)))fail('CONFIG.INVALID_REQUEST');
  for(const k of strings)if(k in patch&&(typeof patch[k]!=='string'||patch[k].length>1024||/[\x00-\x1f\x7f]/.test(patch[k])||['[LOCAL_PATH]','[REDACTED]'].includes(patch[k])))fail('CONFIG.INVALID_REQUEST');
  for(const k of numbers)if(k in patch&&(!Number.isInteger(patch[k])||patch[k]<1||patch[k]>(k==='log_baud'?10000000:k==='rx_i2c_clock_hz'?1000000:300)))fail('CONFIG.INVALID_REQUEST');
  for(const k of booleans)if(k in patch&&typeof patch[k]!=='boolean')fail('CONFIG.INVALID_REQUEST');
  const current=store.withLock(()=>getProject(store));if(!current)fail('PROJECT.REQUIRED');
  if(current.revision!==params.expected_revision||current.sha256!==params.expected_sha256)fail('CONFIG.REVISION_CONFLICT');
  const value=structuredClone(current.value);if(!value.station||typeof value.station!=='object')fail('CONFIG.INVALID_REQUEST');
  Object.assign(value.station,structuredClone(patch));
  if(value.station.scope_source_enabled===true&&value.station.scope_capture_enabled!==true)fail('CONFIG.INVALID_REQUEST','Source waveform requires screenshot capture');
  return saveProject(store,{expected_revision:current.revision,expected_sha256:current.sha256,value});
}
function patchProjectDongleUart(store,params){
  if(store.s.plugin.id!=='fpga-firmware-test'||typeof params.expected_project!=='string'||params.expected_project!==store.s.project)fail('CONFIG.CONTEXT_CHANGED');
  if(typeof params.role!=='string'||!/^[A-Za-z0-9][A-Za-z0-9_-]{0,63}$/.test(params.role)||typeof params.serial!=='string'||!/^[A-Za-z0-9._-]{1,128}$/.test(params.serial)||params.channel!==0||!Number.isInteger(params.baud)||params.baud<1||params.baud>12000000)fail('CONFIG.INVALID_REQUEST');
  const current=store.withLock(()=>getProject(store));if(!current?.saved)fail('PROJECT.REQUIRED');
  if(current.revision!==params.expected_revision||current.sha256!==params.expected_sha256)fail('CONFIG.REVISION_CONFLICT');
  const value=structuredClone(current.value),binding=value.instruments?.[params.role];
  if(!binding||binding.kind!=='wiparse_dongle_uart'||!binding.selector||typeof binding.selector!=='object'||Array.isArray(binding.selector))fail('CONFIG.INVALID_REQUEST');
  binding.selector.dongle_serial=params.serial;binding.selector.uart_channel=params.channel;binding.selector.baud=params.baud;
  return saveProject(store,{expected_revision:current.revision,expected_sha256:current.sha256,value});
}
// Update a limited set of visible item properties without sending redacted
// fields or secrets through the frontend. save() still performs schema checks,
// locking, audit, and revision/hash conflict detection before committing.
function patchItemProperties(store,params){
  if(typeof params.expected_project!=='string'||params.expected_project!==store.s.project)fail('CONFIG.CONTEXT_CHANGED');
  const patch=params.patch;
  if(!patch||typeof patch!=='object'||Array.isArray(patch))fail('CONFIG.INVALID_REQUEST');
  const allowed=new Set(['name','enabled','order','tags','timeout_s','log_capture','waveform_capture','waveform_capture_delete','verdict_rule','verdict_rule_delete','reset','wireless_limits']);
  if(!Object.keys(patch).length||Object.keys(patch).some(key=>!allowed.has(key)))fail('CONFIG.INVALID_REQUEST');
  const row=store.withLock(()=>store.get(params.config_id));
  const value=structuredClone(row.value);
  for(const [key,item] of Object.entries(patch)){
    if(key==='wireless_limits'){
      if(store.s.plugin.id!=='wireless-whitebox'||value.id!=='wcv_07'||!item||typeof item!=='object'||Array.isArray(item)||!Object.keys(item).length||Object.keys(item).some(k=>!['limit','rx_vout_control','physical_vout_limit','load_cc'].includes(k)))fail('CONFIG.INVALID_REQUEST');
      const exact=(obj,keys)=>obj&&typeof obj==='object'&&!Array.isArray(obj)&&Object.keys(obj).length===keys.length&&keys.every(k=>Object.hasOwn(obj,k));
      const finite=v=>typeof v==='number'&&Number.isFinite(v);
      if('limit'in item){const x=item.limit;if(!exact(x,['metric','unit','minimum','maximum'])||!['fop','iout','vout','vrect','temp'].includes(x.metric)||x.unit!=='raw'||!finite(x.minimum)||!finite(x.maximum)||x.minimum>x.maximum)fail('CONFIG.INVALID_REQUEST');}
      if('rx_vout_control'in item){const x=item.rx_vout_control;if(!exact(x,['target_mv','minimum_mv','maximum_mv'])||![x.target_mv,x.minimum_mv,x.maximum_mv].every(v=>Number.isInteger(v)&&v>=0&&v<=65535)||x.minimum_mv>x.target_mv||x.target_mv>x.maximum_mv)fail('CONFIG.INVALID_REQUEST');}
      if('physical_vout_limit'in item){const x=item.physical_vout_limit;if(!exact(x,['minimum_v','maximum_v','settle_ms'])||!finite(x.minimum_v)||!finite(x.maximum_v)||x.minimum_v>x.maximum_v||!Number.isInteger(x.settle_ms)||x.settle_ms<0||x.settle_ms>60000)fail('CONFIG.INVALID_REQUEST');}
      if('load_cc'in item){const x=item.load_cc;if(!exact(x,['current_a','maximum_a'])||!finite(x.current_a)||!finite(x.maximum_a)||x.current_a<0||x.maximum_a<=0||x.current_a>x.maximum_a)fail('CONFIG.INVALID_REQUEST');}
      for(const [name,data] of Object.entries(item))value[name]=structuredClone(data);
    } else if(key==='log_capture'){
      if(!item||typeof item!=='object'||Array.isArray(item)||Object.keys(item).some(k=>!['enabled','start_at_s','duration_s'].includes(k))||!Object.keys(item).length)fail('CONFIG.INVALID_REQUEST');
      if('enabled'in item&&typeof item.enabled!=='boolean')fail('CONFIG.INVALID_REQUEST');
      for(const k of ['start_at_s','duration_s'])if(k in item&&(typeof item[k]!=='number'||!Number.isFinite(item[k])||item[k]<(k==='duration_s'?Number.MIN_VALUE:0)||item[k]>1000000))fail('CONFIG.INVALID_REQUEST');
      if(!value.log_capture||typeof value.log_capture!=='object'||Array.isArray(value.log_capture))fail('CONFIG.INVALID_REQUEST');
      value.log_capture={...value.log_capture,...item};
    } else if(key==='waveform_capture_delete'){
      if(store.s.plugin.id!=='fpga-firmware-test'||!item||typeof item!=='object'||Array.isArray(item)||Object.keys(item).length!==1||typeof item.id!=='string'||!/^[A-Za-z0-9_.-]{1,128}$/.test(item.id)||!Array.isArray(value.waveform_captures))fail('CONFIG.INVALID_REQUEST');
      if(value.waveform_captures.filter(c=>c?.id===item.id).length!==1)fail('CONFIG.INVALID_REQUEST');
      const refs=new Set([item.id,`${item.id}.source`]);
      const inUse=rules=>(Array.isArray(rules)?rules:[]).some(rule=>refs.has(rule?.capture)||refs.has(rule?.evidence_ref)||(Array.isArray(rule?.evidence_refs)&&rule.evidence_refs.some(ref=>refs.has(ref)))||inUse(rule?.rules));
      if(inUse(value.verdict?.rules))fail('CONFIG.CAPTURE_IN_USE','A verdict rule still references this capture');
      value.waveform_captures=value.waveform_captures.filter(c=>c.id!==item.id);
    } else if(key==='waveform_capture'){
      if(!item||typeof item!=='object'||Array.isArray(item)||Object.keys(item).some(k=>!['id','at_s','timeout_s','source_enabled','source_format','channels','create'].includes(k))||typeof item.id!=='string'||!/^[A-Za-z0-9_.-]{1,128}$/.test(item.id)||typeof item.create!=='boolean')fail('CONFIG.INVALID_REQUEST');
      if(typeof item.at_s!=='number'||!Number.isFinite(item.at_s)||item.at_s<0||item.at_s>1000000||typeof item.timeout_s!=='number'||!Number.isFinite(item.timeout_s)||item.timeout_s<=0||item.timeout_s>1000000||typeof item.source_enabled!=='boolean'||!['instrument_native','csv'].includes(item.source_format))fail('CONFIG.INVALID_REQUEST');
      if(!Array.isArray(item.channels)||!item.channels.length||item.channels.length>32||item.channels.some(c=>typeof c!=='string'||!c.trim()||c.length>128||/[\x00-\x1f\x7f]/.test(c))||new Set(item.channels).size!==item.channels.length)fail('CONFIG.INVALID_REQUEST');
      if(!Array.isArray(value.waveform_captures))fail('CONFIG.INVALID_REQUEST');
      const matches=value.waveform_captures.filter(c=>c?.id===item.id);
      if(item.create){
        if(matches.length||value.waveform_captures.length>=500)fail('CONFIG.ID_CONFLICT');
        value.waveform_captures.push({id:item.id,at_s:item.at_s,timeout_s:item.timeout_s,kind:'screenshot',channels:structuredClone(item.channels),source_waveform:{enabled:item.source_enabled,format:item.source_format}});
      }else{
        if(matches.length!==1||!matches[0].source_waveform||typeof matches[0].source_waveform!=='object')fail('CONFIG.INVALID_REQUEST');
        const checkReferences=rules=>{for(const rule of Array.isArray(rules)?rules:[]){if(rule?.type==='waveform_metric_range'&&rule.capture===item.id&&!item.channels.includes(rule.channel))fail('CONFIG.CAPTURE_IN_USE','A verdict rule still references a removed channel');checkReferences(rule?.rules);}};
        checkReferences(value.verdict?.rules);
        const capture=matches[0];capture.at_s=item.at_s;capture.timeout_s=item.timeout_s;capture.channels=structuredClone(item.channels);capture.source_waveform={...capture.source_waveform,enabled:item.source_enabled,format:item.source_format};
      }
    } else if(key==='reset'){
      if(!item||typeof item!=='object'||Array.isArray(item)||Object.keys(item).some(k=>!['method','role','channel','off_time_ms'].includes(k))||!['dc_power','wiparse_dongle'].includes(item.method)||typeof item.role!=='string'||!item.role||item.role.length>64||!Number.isInteger(item.channel)||item.channel<1||item.channel>256||typeof item.off_time_ms!=='number'||!Number.isFinite(item.off_time_ms)||item.off_time_ms<0||item.off_time_ms>1000000)fail('CONFIG.INVALID_REQUEST');
      const project=store.withLock(()=>getProject(store));if(!project?.saved||project.revision!==params.expected_project_revision||project.sha256!==params.expected_project_sha256)fail('CONFIG.REVISION_CONFLICT');
      const binding=project.value?.instruments?.[item.role],kinds=item.method==='dc_power'?['dc_source']:['debug_probe','wiparse_dongle'];
      if(!binding||!kinds.includes(binding.kind)||!value.reset?.release_confirmation)fail('CONFIG.INVALID_REQUEST');
      const confirmation=structuredClone(value.reset.release_confirmation);
      value.reset=item.method==='dc_power'?{method:item.method,power_role:item.role,channel:item.channel,off_time_ms:item.off_time_ms,release_confirmation:confirmation}:{method:item.method,dongle_role:item.role,hardware:true,release_confirmation:confirmation};
      params={...params,project_snapshot:{revision:project.revision,sha256:project.sha256}};
    } else if(key==='verdict_rule_delete'){
      if(store.s.plugin.id!=='fpga-firmware-test'||!item||typeof item!=='object'||Array.isArray(item)||Object.keys(item).length!==1||typeof item.id!=='string'||!item.id||item.id.length>128||/[\x00-\x1f\x7f]/.test(item.id)||!Array.isArray(value.verdict?.rules))fail('CONFIG.INVALID_REQUEST');
      if(value.verdict.rules.filter(rule=>rule?.id===item.id).length!==1)fail('CONFIG.INVALID_REQUEST');
      if(value.verdict.rules.length<=1)fail('CONFIG.LAST_RULE_REQUIRED');
      value.verdict.rules=value.verdict.rules.filter(rule=>rule.id!==item.id);
    } else if(key==='verdict_rule'){
      if(!item||typeof item!=='object'||Array.isArray(item)||typeof item.id!=='string'||!item.id||item.id.length>128||typeof item.type!=='string'||typeof item.required!=='boolean'||typeof item.create!=='boolean')fail('CONFIG.INVALID_REQUEST');
      const matches=Array.isArray(value.verdict?.rules)?value.verdict.rules.filter(rule=>rule?.id===item.id):[];
      if(!Array.isArray(value.verdict?.rules)||item.create&&matches.length||!item.create&&(matches.length!==1||matches[0].type!==item.type))fail('CONFIG.INVALID_REQUEST');
      if(item.create){const hasId=rules=>(Array.isArray(rules)?rules:[]).some(rule=>rule?.id===item.id||hasId(rule?.rules));if(hasId(value.verdict.rules))fail('CONFIG.ID_CONFLICT');}
      if(item.create&&!['log_contains','log_regex','log_not_regex'].includes(item.type))fail('CONFIG.INVALID_REQUEST');
      const rule=item.create?{id:item.id,type:item.type}:matches[0];let allowedFields;
      if(item.type==='log_contains')allowedFields=['id','type','required','text','create'];
      else if(['log_regex','log_not_regex'].includes(item.type))allowedFields=['id','type','required','pattern','create'];
      else if(['waveform_metric_range','instrument_measure_range'].includes(item.type))allowedFields=['id','type','required','minimum','maximum','create'];
      else fail('CONFIG.INVALID_REQUEST');
      if(Object.keys(item).some(k=>!allowedFields.includes(k))||Object.keys(item).length!==allowedFields.length)fail('CONFIG.INVALID_REQUEST');
      if('text'in item||'pattern'in item){const field='text'in item?'text':'pattern';if(typeof item[field]!=='string'||!item[field]||item[field].length>4096)fail('CONFIG.INVALID_REQUEST');rule[field]=item[field];}
      else {if(typeof item.minimum!=='number'||typeof item.maximum!=='number'||!Number.isFinite(item.minimum)||!Number.isFinite(item.maximum)||item.minimum>item.maximum)fail('CONFIG.INVALID_REQUEST');rule.minimum=item.minimum;rule.maximum=item.maximum;}
      rule.required=item.required;
      if(item.create){if(value.verdict.rules.length>=500)fail('CONFIG.REPOSITORY_LIMIT');value.verdict.rules.push(rule);}
    } else value[key]=structuredClone(item);
  }
  return store.save({...params,value},'patch');
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

function projectData(store){const record=getProject(store);if(!record)return null;return {...record,schema:store.projectDefinition.schema.schema,documents:store.projectDefinition.schema.documents,ui:store.projectDefinition.ui};}
