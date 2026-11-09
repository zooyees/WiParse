import path from 'node:path';
import fs from 'node:fs';
import {createHash,randomUUID} from 'node:crypto';
import {atomicWrite,jail} from '../../lib/hub/common.mjs';
import {inspectTerminalExport,findAlertRecoveryCandidates} from './mt5820-log.mjs';
import {inspectStation} from './station-readiness.mjs';
import {resolveBridge,observeRx} from './rx-observe.mjs';

function inputLog(ctx) {
  if (!ctx.args?.log_file) throw new Error('LOG.FILE_REQUIRED');
  const root=fs.realpathSync(ctx.dataRoot), file=jail(path.resolve(root,ctx.args.log_file),[root]);
  const stat=fs.statSync(file);
  if (!stat.isFile() || stat.size>16*1024*1024) throw new Error('LOG.INVALID_SIZE_OR_TYPE');
  const bytes=fs.readFileSync(file);
  return {bytes,sha256:createHash('sha256').update(bytes).digest('hex')};
}
function validRxLimit(limit) {
  return !limit || (['fop','iout','vout','vrect','temp'].includes(limit.metric)&&limit.unit==='raw'&&Number.isFinite(limit.minimum)&&Number.isFinite(limit.maximum)&&limit.minimum<=limit.maximum);
}
function validRxControl(c) {
  return !!c && [c.target_mv,c.minimum_mv,c.maximum_mv].every(v=>Number.isInteger(v)&&v>=0&&v<=65535) && c.minimum_mv<=c.target_mv && c.target_mv<=c.maximum_mv;
}
function validPhysicalLimit(c) {
  return !c || (Number.isFinite(c.minimum_v)&&Number.isFinite(c.maximum_v)&&c.minimum_v<=c.maximum_v&&Number.isInteger(c.settle_ms)&&c.settle_ms>=0&&c.settle_ms<=60000);
}
function validLoadCc(c) {
  return !!c && Number.isFinite(c.current_a) && Number.isFinite(c.maximum_a) && c.current_a>=0 && c.maximum_a>0 && c.current_a<=c.maximum_a;
}
function recordRxFailure(ctx,b,error) {
  if(!ctx.project||!ctx.dataRoot||!b.items||!b.project_config)return;
  const item=b.items.find(v=>v.id==='wcv_07');if(!item?.sha256)return;
  const code=String(error?.code||error?.message||'RX.ERROR').split(':')[0].replace(/[^A-Za-z0-9._-]/g,'_').slice(0,128);
  const resultPath=path.join(ctx.layout.artifacts_dir,'reports','items','wcv_07','result.json');
  const config=b.values.find(v=>v.id==='wcv_07'), partial=error?.partial_observation||{},checks=[];
  if(config?.limit&&validRxLimit(config.limit)) {
    const value=partial.telemetry?.values_raw?.[config.limit.metric];
    if(Number.isFinite(value))checks.push({id:'rx_raw_range',metric:config.limit.metric,value,unit:'raw',minimum:config.limit.minimum,maximum:config.limit.maximum,verdict:value>=config.limit.minimum&&value<=config.limit.maximum?'PASS':'FAIL',scope:'internal_register_only'});
  }
  if(config?.physical_vout_limit&&validPhysicalLimit(config.physical_vout_limit)) {
    const value=partial.physical_measurement?.value,limit=config.physical_vout_limit;
    if(Number.isFinite(value))checks.push({id:'rx_vout_instrument',metric:'rx_vout',value,unit:'V',minimum:limit.minimum_v,maximum:limit.maximum_v,verdict:value>=limit.minimum_v&&value<=limit.maximum_v?'PASS':'FAIL',scope:'instrument_reading_only'});
  }
  const failed=checks.some(c=>c.verdict==='FAIL');
  const ref=path.relative(path.join(ctx.dataRoot,'projects',ctx.project),resultPath).split(path.sep).join('/');
  const attempt_id=randomUUID();
  const rows=b.values.map(v=>({id:v.id,name:v.name,tags:v.tags||[],order:v.order||0,selected:v.id==='wcv_07',status:v.id==='wcv_07'?(failed?'fail':'error'):'skipped',phase:'stopped',attempt:1,progress:0,elapsed_s:0,message:v.id==='wcv_07'?code:'未选择',result_ref:v.id==='wcv_07'?ref:null,...(v.id==='wcv_07'?{attempt_id,config_sha256:item.sha256,error_code:code}:{})}));
  const summary={total:rows.length,selected:1};
  for(const state of ['pending','running','pass','fail','error','skipped','aborted','inconclusive'])summary[state]=rows.filter(r=>r.status===state).length;
  const artifacts={};
  if(Object.keys(partial).length){atomicWrite(path.join(path.dirname(resultPath),'partial-observation.json'),JSON.stringify(partial,null,2));artifacts.partial_observation={path:'partial-observation.json'};}
  atomicWrite(resultPath,JSON.stringify({schema_version:'wiparse.wireless-result/1',scope:{project_id:ctx.project,test_id:path.basename(ctx.layout.test_dir),plugin_id:ctx.plugin.id},item_id:'wcv_07',run_id:b.run_id,attempt:1,attempt_id,config_sha256:item.sha256,project_config:b.project_config,execution_status:'error',product_verdict:failed?'FAIL':'INCONCLUSIVE',message:code,checks,evidence:{artifacts}},null,2));
  atomicWrite(ctx.matrixFile,JSON.stringify({schema_version:'wiparse.test-matrix/1',plugin_id:ctx.plugin.id,revision:1,run_id:b.run_id,preflight_revision:b.preflight_revision,updated_at:new Date().toISOString(),active_item_id:null,summary,items:rows,project_config:b.project_config}));
}

export async function configure(ctx) {
  const value=ctx.overlay?.config;
  if (!value || typeof value!=='object') return {ok:false,lifecycle:'configure',errors:[{path:'/',message:'需要测试项配置'}],warnings:[]};
  const errors=[];
  if(value.limit && Number.isFinite(value.limit.minimum) && Number.isFinite(value.limit.maximum) && value.limit.minimum>value.limit.maximum)
    errors.push({path:'/limit',message:'测量下限不能大于上限'});
  if(value.rx_vout_control && !validRxControl(value.rx_vout_control))errors.push({path:'/rx_vout_control',message:'目标电压必须在允许范围内'});
  if(!validPhysicalLimit(value.physical_vout_limit))errors.push({path:'/physical_vout_limit',message:'电压范围或等待时间无效'});
  if(value.load_cc && !validLoadCc(value.load_cc))errors.push({path:'/load_cc',message:'负载电流必须在配置的允许范围内'});
  return {ok:!errors.length, lifecycle:'configure', errors, warnings:[], normalized_value:null};
}

export async function configureProject() {
  return {ok:true, lifecycle:'configure', errors:[], warnings:[{path:'/',message:'设备标识仅记录，实物连接与能力校验尚未实现'}]};
}

export async function preflight(ctx) {
  if (ctx.args?.mode === 'live') {
    const readiness=await inspectStation(ctx);
    return {ok:false,checks:readiness.checks,error:'WIRELESS.LIVE_BACKEND_NOT_VALIDATED: 实物采集和刺激尚未接通'};
  }
  const b = ctx.testMatrix;
  if (!b?.selected_item_ids?.length) return {ok:false, error:'请选择测试项'};
  if (['rx_read','rx_control','rx_load'].includes(ctx.args?.mode)) {
    if(b.selected_item_ids.length!==1 || b.selected_item_ids[0]!=='wcv_07')return {ok:false,error:'读取 MT5788 状态时请仅选择“低功率稳态”测试项'};
    const selected=b.values?.find(v=>v.id==='wcv_07');
    if(!validRxLimit(selected?.limit))return {ok:false,error:'RX.LIMIT_INVALID：RX 原始值检查的项目须为 fop/iout/vout/vrect/temp，单位须为 raw 且上下限有效'};
    if(!validPhysicalLimit(selected?.physical_vout_limit))return {ok:false,error:'LOAD.LIMIT_INVALID：请检查物理电压范围及等待时间'};
    if(ctx.args.mode==='rx_control' && !validRxControl(selected?.rx_vout_control))return {ok:false,error:'RX.VOUT_LIMIT_REQUIRED：请填写目标值和允许范围'};
    if(ctx.args.mode==='rx_control' && !selected?.physical_vout_limit)return {ok:false,error:'LOAD.LIMIT_REQUIRED：请配置电子负载端电压检查'};
    if(ctx.args.mode==='rx_load' && !validLoadCc(selected?.load_cc))return {ok:false,error:'LOAD.CURRENT_LIMIT_REQUIRED：请填写电流和允许上限'};
    if(ctx.args.mode==='rx_load' && !selected?.physical_vout_limit)return {ok:false,error:'LOAD.LIMIT_REQUIRED：请配置电子负载端电压检查'};
    try {await resolveBridge(ctx);}catch(e){return {ok:false,error:`RX.PREFLIGHT: ${e.message}`};}
  }
  if (ctx.args?.mode==='log_analysis') {
    if (!b.selected_item_ids.includes('wcv_20')) return {ok:false,error:'日志分析请选择历史故障复现测试项'};
    try {const log=inputLog(ctx);atomicWrite(path.join(ctx.layout.test_dir,'wireless-log-preflight.json'),JSON.stringify({sha256:log.sha256,manifest:b.manifest_sha256}));}
    catch(e) {return {ok:false,error:`LOG.PREFLIGHT: ${e.message}`};}
  }
  return {ok:true, test_matrix:{manifest_sha256:b.manifest_sha256, revision:b.manifest_sha256, item_count:b.selected_item_ids.length}, warnings:[['rx_read','rx_control','rx_load'].includes(ctx.args?.mode)?'运行后需观察到本轮 TX 新的 RX 运行报文或建链序列才访问 RX；不能单凭内部数据判定产品':'不连接设备或判定产品']};
}

export async function stop(ctx) {
  const dir=ctx.layout?.test_dir;
  if(!dir)return {ok:true};
  const active=path.join(dir,'wireless-rx-active.json');
  if(!fs.existsSync(active))return {ok:true};
  const record=JSON.parse(fs.readFileSync(active,'utf8'));
  if(record.run_id)atomicWrite(path.join(dir,'wireless-rx-stop.json'),JSON.stringify({run_id:record.run_id}));
  return {ok:true};
}

export default async function run(ctx) {
  if (ctx.args?.mode === 'live') throw new Error('WIRELESS.LIVE_BACKEND_NOT_VALIDATED');
  const b = ctx.testMatrix;
  if (!b?.values || !b?.selected_item_ids) throw new Error('MATRIX.BINDING_REQUIRED');
  let rxObservation=null,rxCheck=null,physicalCheck=null;
  if(['rx_read','rx_control','rx_load'].includes(ctx.args?.mode)) {
    if(b.selected_item_ids.length!==1 || b.selected_item_ids[0]!=='wcv_07')throw new Error('RX.INVALID_SELECTION');
    if(!validRxLimit(b.values.find(v=>v.id==='wcv_07')?.limit))throw new Error('RX.LIMIT_INVALID');
    if(!validPhysicalLimit(b.values.find(v=>v.id==='wcv_07')?.physical_vout_limit))throw new Error('LOAD.LIMIT_INVALID');
    const control=b.values.find(v=>v.id==='wcv_07')?.rx_vout_control;
    if(ctx.args.mode==='rx_control'&&!validRxControl(control))throw new Error('RX.VOUT_LIMIT_REQUIRED');
    if(ctx.args.mode==='rx_control'&&!b.values.find(v=>v.id==='wcv_07')?.physical_vout_limit)throw new Error('LOAD.LIMIT_REQUIRED');
    const load=b.values.find(v=>v.id==='wcv_07')?.load_cc;
    if(ctx.args.mode==='rx_load'&&!validLoadCc(load))throw new Error('LOAD.CURRENT_LIMIT_REQUIRED');
    if(ctx.args.mode==='rx_load'&&!b.values.find(v=>v.id==='wcv_07')?.physical_vout_limit)throw new Error('LOAD.LIMIT_REQUIRED');
    const active=path.join(ctx.layout.test_dir,'wireless-rx-active.json'),stopFile=path.join(ctx.layout.test_dir,'wireless-rx-stop.json');
    const fd=fs.openSync(active,'wx');try{fs.writeFileSync(fd,JSON.stringify({run_id:b.run_id}));}finally{fs.closeSync(fd);}
    const stopped=()=>{try{return JSON.parse(fs.readFileSync(stopFile,'utf8')).run_id===b.run_id;}catch{return false;}};
    try {rxObservation=await observeRx(ctx,{stopped,control:ctx.args.mode==='rx_control'?control:null,load:ctx.args.mode==='rx_load'?load:null});}
    catch(error) {try{recordRxFailure(ctx,b,error);}catch(e){error.evidence_error=e.message;}throw error;}
    finally {
      try {if(JSON.parse(fs.readFileSync(active,'utf8')).run_id===b.run_id)fs.unlinkSync(active);}catch{}
      try {if(JSON.parse(fs.readFileSync(stopFile,'utf8')).run_id===b.run_id)fs.unlinkSync(stopFile);}catch{}
    }
    const limit=b.values.find(v=>v.id==='wcv_07')?.limit;
    if(limit) {
      const value=rxObservation.telemetry.values_raw[limit.metric];
      if(limit.unit!=='raw'||!Number.isFinite(value)||!Number.isFinite(limit.minimum)||!Number.isFinite(limit.maximum)||limit.minimum>limit.maximum)throw new Error('RX.LIMIT_INVALID');
      rxCheck={id:'rx_raw_range',metric:limit.metric,value,unit:'raw',minimum:limit.minimum,maximum:limit.maximum,verdict:value>=limit.minimum&&value<=limit.maximum?'PASS':'FAIL',scope:'internal_register_only'};
      rxObservation.check=rxCheck;
      if(rxCheck.verdict==='FAIL')rxObservation.product_verdict='FAIL';
    }
    const physicalLimit=b.values.find(v=>v.id==='wcv_07')?.physical_vout_limit;
    if(physicalLimit) {
      const value=rxObservation.physical_measurement?.value;
      if(!Number.isFinite(value))throw new Error('LOAD.VOLTAGE_INVALID');
      physicalCheck={id:'rx_vout_instrument',metric:'rx_vout',value,unit:'V',minimum:physicalLimit.minimum_v,maximum:physicalLimit.maximum_v,verdict:value>=physicalLimit.minimum_v&&value<=physicalLimit.maximum_v?'PASS':'FAIL',scope:'instrument_reading_only'};
      rxObservation.physical_check=physicalCheck;
      if(physicalCheck.verdict==='FAIL')rxObservation.product_verdict='FAIL';
    }
    atomicWrite(path.join(ctx.layout.artifacts_dir,'reports','rx-telemetry.json'),JSON.stringify(rxObservation,null,2));
  }
  let logAnalysis=null;
  if(ctx.args?.mode==='log_analysis') {
    const input=inputLog(ctx), binding=JSON.parse(fs.readFileSync(path.join(ctx.layout.test_dir,'wireless-log-preflight.json'),'utf8'));
    if(binding.sha256!==input.sha256 || binding.manifest!==b.manifest_sha256) throw new Error('LOG.PREFLIGHT_STALE');
    const content=new TextDecoder('utf-8',{fatal:true}).decode(input.bytes);
    const observations=inspectTerminalExport(content), candidates=findAlertRecoveryCandidates(observations.events);
    const folder=path.join(ctx.layout.artifacts_dir,'reports');
    atomicWrite(path.join(folder,'source-log.txt'),input.bytes);
    logAnalysis={source_sha256:input.sha256,blocks:observations.blocks,candidates,limitations:observations.limitations,product_verdict:'INCONCLUSIVE'};
    atomicWrite(path.join(folder,'log-analysis.json'),JSON.stringify(logAnalysis,null,2));
  }
  const selected = new Set(b.selected_item_ids);
  const productVerdict=rxCheck?.verdict==='FAIL'||physicalCheck?.verdict==='FAIL'?'FAIL':'INCONCLUSIVE';
  const items = b.values.map(v => ({id:v.id, name:v.name, tags:v.tags || [], order:v.order || 0, selected:selected.has(v.id), status:selected.has(v.id)?(v.id==='wcv_07'&&productVerdict==='FAIL'?'fail':'inconclusive'):'skipped', phase:'review', attempt:1, progress:1, elapsed_s:0, message:selected.has(v.id)?(ctx.args?.mode==='log_analysis'&&v.id==='wcv_20'?'日志已分析，实物结果无法判定':rxObservation&&v.id==='wcv_07'?(productVerdict==='FAIL'?'RX 检查超出配置限值':'已读取 RX 状态，完整测试未完成'):'仅检查配置，未连接设备'):'未选择', result_ref:null}));
  // The Hub result-details contract binds results to this exact run, item,
  // attempt, input configuration and shared engineering settings.
  if (ctx.project && ctx.dataRoot && b.items && b.project_config) {
    const projectRoot=path.join(ctx.dataRoot,'projects',ctx.project);
    for (const row of items.filter(r=>r.selected)) {
      const input=b.items.find(item=>item.id===row.id);
      if (!input?.sha256) throw new Error('RESULT.CONFIG_IDENTITY_REQUIRED');
      const dir=path.join(ctx.layout.artifacts_dir,'reports','items',row.id);
      const resultPath=path.join(dir,'result.json');
      row.attempt_id=randomUUID();row.config_sha256=input.sha256;
      row.result_ref=path.relative(projectRoot,resultPath).split(path.sep).join('/');
      const artifacts={};
      if(logAnalysis && row.id==='wcv_20') {
        atomicWrite(path.join(dir,'log-analysis.json'),JSON.stringify(logAnalysis,null,2));
        artifacts.analysis={path:'log-analysis.json'};
      }
      if(rxObservation && row.id==='wcv_07') {
        atomicWrite(path.join(dir,'rx-telemetry.json'),JSON.stringify(rxObservation,null,2));
        artifacts.rx_telemetry={path:'rx-telemetry.json'};
        for(const [name,value] of Object.entries(rxObservation.captures||{}))artifacts[name]=value;
      }
      atomicWrite(resultPath,JSON.stringify({schema_version:'wiparse.wireless-result/1',scope:{project_id:ctx.project,test_id:path.basename(ctx.layout.test_dir),plugin_id:ctx.plugin.id},item_id:row.id,run_id:b.run_id,attempt:row.attempt,attempt_id:row.attempt_id,config_sha256:row.config_sha256,project_config:b.project_config,execution_status:'completed',product_verdict:row.id==='wcv_07'?productVerdict:'INCONCLUSIVE',message:row.message,evidence:{artifacts},...(logAnalysis&&row.id==='wcv_20'?{candidate_count:logAnalysis.candidates.length}: {}),...((rxCheck||physicalCheck)&&row.id==='wcv_07'?{checks:[rxCheck,physicalCheck].filter(Boolean)}:{})},null,2));
    }
  }
  const summary = {total:items.length, selected:items.filter(i=>i.selected).length};
  for (const status of ['pending','running','pass','fail','error','skipped','aborted','inconclusive']) summary[status]=items.filter(i=>i.status===status).length;
  atomicWrite(ctx.matrixFile, JSON.stringify({schema_version:'wiparse.test-matrix/1',plugin_id:ctx.plugin.id,revision:1,run_id:b.run_id,preflight_revision:b.preflight_revision,updated_at:new Date().toISOString(),active_item_id:null,summary,items,...(b.project_config?{project_config:b.project_config}:{})}));
  const report = path.join(ctx.layout.artifacts_dir,'reports','summary.md');
  const rows = b.values.filter(v=>selected.has(v.id)).map(v=>`## ${v.name}\n\n- 测试条件：${v.stimulus}\n- 内部证据：${v.internal_evidence}\n- 电气证据：${v.physical_evidence}\n- 判定要求：${v.judgment}\n- 测量限值：${v.limit ? `${v.limit.metric}：${v.limit.minimum} 至 ${v.limit.maximum} ${v.limit.unit}` : '未配置'}\n- 结果：${v.id==='wcv_07'&&productVerdict==='FAIL'?'FAIL（配置的检查项超出限值）':'无法判定（完整测试未完成）'}`);
  const project = ctx.projectConfig?.value || {};
  const diagnosis=logAnalysis?'\n## 历史故障日志分析\n\n'+
    `日志 SHA-256：${logAnalysis.source_sha256}；识别到 ${logAnalysis.candidates.length} 次 FO Alert 候选。\n\n`+
    logAnalysis.candidates.map((c,i)=>`${i+1}. 接收时间 ${c.alert.receive_time}，前次 FodCnt=${c.prior_count?.fields.raw_value ?? '未知'}，后续 EPT_REASON=${c.reason?.fields.raw_value ?? '未知'}，后续 Start CHG=${c.next_charge?.receive_time ?? '未观察到'}。`).join('\n\n')+
    '\n\n结果：无法判定。终端时间为接收块时间，字段可能被分包截断；具体因果、规则和电气行为需实测确认。\n\n[日志分析数据](log-analysis.json) | [输入日志](source-log.txt)\n':'\n';
  const rxSection=rxObservation?'\n## MT5788 RX 寄存器观察\n\n'+
    `FT4222：${rxObservation.source_resource}；芯片：${rxObservation.identity.variant}；SYS_MODE 原始值：${rxObservation.telemetry.sys_mode_raw}。\n\n`+
    `本轮 TX 日志依次观察到：${rxObservation.readiness.events.map(e=>e.marker.replace(/[\r\n]/g,' ')).join(' → ')}（主机轮询观察，非设备事件时间）。\n\n`+
    (rxObservation.control?`已请求设置 RX 输出 ${rxObservation.control.requested_mv} mV；设置命令本身不证明实际电压。\n\n`:'')+
    (rxObservation.load?`电子负载恒流设定 ${rxObservation.load.requested_current_a} A，允许上限 ${rxObservation.load.maximum_current_a} A；已确认关断命令完成，物理关断状态未验证。\n\n`:'')+
    Object.entries(rxObservation.telemetry.values_raw).map(([key,value])=>`- ${key}：${value}（原始值，单位未确认）`).join('\n')+
    (rxCheck?`\n\n内部寄存器检查：${rxCheck.metric}=${rxCheck.value}，限值 ${rxCheck.minimum} 至 ${rxCheck.maximum} raw，结果 ${rxCheck.verdict}。该检查不代表电气或产品结果。\n`:'\n\n未配置内部寄存器限值，本次不作数值检查。\n')+
    (physicalCheck?`\n\n电子负载报告的 RX VOUT：${physicalCheck.value} V，限值 ${physicalCheck.minimum} 至 ${physicalCheck.maximum} V，单次检查 ${physicalCheck.verdict}；实际量程、校准和稳定时间需确认。\n`:'')+
    (Object.keys(rxObservation.captures||{}).length?'\n示波器证据在寄存器读取后采集，不能视为与协议事件同步：\n\n'+Object.entries(rxObservation.captures).map(([name,a])=>`- [${name}](items/wcv_07/${a.path})`).join('\n')+'\n':'')+
    '\n\n[RX 原始寄存器数据](rx-telemetry.json)\n':'';
  atomicWrite(report, '# 无线充电白盒测试\n\n'+(rxObservation?(rxObservation.load?'本次施加恒流负载并采集 RX 数据，完整产品测试未完成。':rxObservation.control?'本次设置 RX 目标电压并采集内部寄存器，完整产品测试未完成。':'本次仅采集 RX 内部寄存器，完整产品测试未完成。'):'本次未连接设备，不能判定产品。')+'\n\n'+
    `工程：${project.name || '未填写'}\n\n本轮产品序列号：${ctx.args?.dut_serial || '未填写'}\n\n本轮固件版本：${ctx.args?.firmware_version || '未填写'}\n\n`+rows.join('\n\n')+diagnosis+rxSection+'\n');
  return {ok:true, summary_md:report, execution_status:'completed', product_verdict:productVerdict, test_matrix:{manifest_sha256:b.manifest_sha256}};
}
