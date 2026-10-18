import fs from 'node:fs';
import path from 'node:path';
import { randomUUID } from 'node:crypto';
import { atomic, append, digest, fail, utc, bool, hash, jailed } from './lib/common.mjs';
import readline from 'node:readline';
import { flattenRules } from './lib/rule-contract.mjs';
import { guardedAdapter } from './lib/io-guard.mjs';
import { beginReport, appendReport } from './lib/report.mjs';
import { pauseBoundary } from './lib/run-control.mjs';
import { evaluateBounded } from './lib/rule-executor.mjs';
import { discover, unchanged, validateCase } from './lib/config-loader.mjs';
import { resolveResources } from './lib/resource-resolver.mjs';
import { hostAdapter } from './lib/host-adapter.mjs';
import {simulationAdapter} from './lib/simulation-adapter.mjs';
import {checkedProject,projectMetadata,resolveProjectCase} from './lib/project-config.mjs';
export {planProjectMigration} from './lib/project-migration.mjs';
import { clock as defaultClock, resetDut, capture } from './lib/scheduler.mjs';
import { validateAI, advise } from './lib/ai-adapter.mjs';
const read = p => JSON.parse(fs.readFileSync(p,'utf8'));
function paths(ctx) {
  const test=ctx.layout?.test_dir ?? ctx.testDir;
  if(ctx.args?.config_dir&&!path.isAbsolute(ctx.args.config_dir))ctx.args.config_dir=path.resolve(ctx.dataRoot??process.cwd(),ctx.args.config_dir);
  if(!test || !ctx.args?.config_dir) throw fail('CONFIG.PATH_REQUIRED','test_dir and config_dir');
  return {test,manifest:path.join(test,'fpga-preflight.json'),matrix:path.join(test,'matrix.json'),stop:path.join(test,'fpga.stop')};
}
export async function configure(ctx) {
  const value=ctx.overlay?.config;
  if(!value||typeof value!=='object')return {ok:false,lifecycle:'configure',errors:[{path:'/',message:'Configuration object required'}],warnings:[]};
  let errors=[];try{errors=validateCase(resolveProjectCase(value,ctx.projectConfig??ctx.overlay?.project_config).value);}catch(e){errors=e.errors??[{path:'/project_config',code:e.code,message:e.message}];}
  return {ok:!errors.length,lifecycle:'configure',errors,warnings:value.firmware?.download?[{path:'/firmware/download',message:'Firmware download is reserved and will block preflight'}]:[],normalized_value:null};
}
export async function configureProject(ctx){try{checkedProject(ctx.overlay?.project_config??ctx.projectConfig);return {lifecycle:'configure',ok:true,errors:[],warnings:[]};}catch(e){return {lifecycle:'configure',ok:false,errors:e.errors??[{path:'/',code:e.code,message:e.message}],warnings:[]};}}
function adapter(ctx){if(!ctx.fpgaAdapter){const simulated=simulationAdapter(ctx);if(simulated)ctx.fpgaAdapter=simulated;}return hostAdapter(ctx);}
function assertInputs(frozen,ctx){
  const project=checkedProject(ctx.projectConfig,{projectId:ctx.project,testId:path.basename(paths(ctx).test)});
  if(project.sha256!==frozen.project_config?.sha256)throw fail('PROJECT.CHANGED','工程共享配置与本轮冻结值不同');
  const cache=ctx._fpgaInputCache??=new Map();
  const check=(file,expected,json=false)=>{
    const stat=fs.statSync(file,{bigint:true}),signature=`${stat.dev}:${stat.ino}:${stat.size}:${stat.mtimeNs}:${stat.ctimeNs}`;
    const prior=cache.get(file);
    if(prior?.signature===signature&&prior.expected===expected)return;
    const actual=json?digest(read(file)):hashFile(file);
    if(actual!==expected)throw fail('CONFIG.CHANGED',file);
    cache.set(file,{signature,expected});
  };
  if(ctx.configSnapshotDir && frozen.host_manifest_sha256!==ctx.testMatrix?.manifest_sha256)throw fail('RUN.PREFLIGHT_STALE','Host manifest changed');
  for(const item of frozen.items.filter(i=>i.selected)){
    if(ctx.configSnapshotDir){
      const snapshot=path.join(ctx.configSnapshotDir,item.config.id+'.json');
      if(!cache.has(snapshot)){const bound=ctx.testMatrix.values.find(c=>c.id===item.config.id);if(digest(bound)!==item.input_sha256)throw fail('CONFIG.CHANGED',item.config.id);}
      check(snapshot,item.input_sha256,true);
    }else check(item.file,item.config_sha256);
    check(item.firmware.absolute,item.firmware.sha256);
  }
}
const hashFile=file=>hash(fs.readFileSync(file));
async function readJournal(file,remember){
  let tail;
  const lines=readline.createInterface({input:fs.createReadStream(file),crlfDelay:Infinity});
  const accept=line=>{if(line.trim())remember(JSON.parse(line));};
  for await(const line of lines){if(tail!==undefined)accept(tail);tail=line;}
  if(tail!==undefined)try{accept(tail);}catch(error){
    // A crash or disk-full write may leave one unterminated record. Never ignore corrupt complete records.
    const size=fs.statSync(file).size,fd=fs.openSync(file,'r'),last=Buffer.alloc(1);
    try{if(size)fs.readSync(fd,last,0,1,size-1);}finally{fs.closeSync(fd);}
    if(last[0]===10)throw error;
  }
}
async function previousRun(ctx,args,p) {
  if((args.resume_mode??'fresh')==='fresh') return null;
  let value=ctx.previousFpgaRun, file,runRoot;
  if(!value) {
    if(!args.resume_from) throw fail('RUN.RESUME_SOURCE_REQUIRED','请选择历史 suite-result.json');
    file=jailed(path.join(p.test,'runs'),path.resolve(args.resume_from));
    if(fs.statSync(file).isDirectory())file=jailed(file,'artifacts/docs/configuration-manifest.json');
    if(fs.statSync(file).size>16*1048576) throw fail('RUN.RESUME_SOURCE_INVALID');
    value=read(file);
    runRoot=path.resolve(path.dirname(file),'../..');
    const stateFile=path.join(runRoot,'artifacts/docs/run-state.json');
    const state=fs.existsSync(stateFile)?read(stateFile):{};
    if(state.status==='running'&&state.pid){let alive=false;try{process.kill(state.pid,0);alive=true;}catch{}if(alive)throw fail('RUN.RESUME_ACTIVE');}
    if(state.lease_retained)throw fail('RUN.RECOVERY_REQUIRED','旧运行的设备状态尚未确认安全');
    if(path.basename(file)==='configuration-manifest.json')value={content_sha256:value.content_sha256,identity:value.identity,run_id:state.run_id,results:[]};
    if(state.archive_complete===false)value.archive_complete=false;
  }
  const latest=new Map((value.results??[]).map(r=>[r.item_id,r]));
  const remember=entry=>{const before=latest.get(entry.item_id);if(!before||before.attempt_id===entry.attempt_id||(entry.started_at??'')>=(before.started_at??''))latest.set(entry.item_id,entry);};
  if(runRoot){const started=path.join(runRoot,'artifacts/docs/attempt-started.jsonl');if(fs.existsSync(started))await readJournal(started,remember);}
  if(file && value.attempt_index_ref) {
    const index=jailed(path.dirname(file),value.attempt_index_ref);
    await readJournal(index,remember);
  }
  if(runRoot&&!value.attempt_index_ref){const index=path.join(runRoot,'artifacts/reports/attempt-index.jsonl');if(fs.existsSync(index))await readJournal(index,remember);}
  return {value,latest};
}
function contentHash(frozen) {
  const {input_at,...identity}=frozen.identity;
  return digest({items:frozen.items.map(i=>({input_sha256:i.input_sha256,config:i.config,firmware_sha256:i.firmware.sha256})),project_config:frozen.project_config,identity,station:frozen.station});
}
function publish(p,m) {
  m.revision++; m.updated_at=utc();
  m.summary={total:m.items.length,...Object.fromEntries(['pending','running','pass','fail','error','skipped','aborted','inconclusive'].map(s=>[s,m.items.filter(i=>i.status.toLowerCase()===s).length]))};
  for(const row of m.items){row.status=row.status.toLowerCase();delete row.firmware_sha256;}
  m.plugin_id='fpga-firmware-test';m.summary.selected=m.items.filter(i=>i.selected).length;
  const {cycle,elapsed_s,attempt_id,...snapshot}=m;
  atomic(p.matrix,snapshot); atomic(path.join(p.test,'status.json'),{step:m.active_item_id??'suite',hint:'FPGA固件测试',cycle:m.cycle??0,elapsed_s:m.elapsed_s??0});
}
function identity(args) {
  const simulation=args.execution_mode==='simulation';
  const result={identity_source:simulation?'simulation':'user_input',operator:args.operator??'unspecified',input_at:utc(),field_sources:{operator:args.operator?'run_parameters':'default_unspecified'}};
  for(const key of ['dut_serial','board_revision','fpga_model','fixture_id','station_id']) {
    if(typeof args[key]!=='string'||!args[key].trim()){if(!simulation)throw fail('DUT.IDENTITY_REQUIRED',key);result[key]=`SIM-${key}`;result.field_sources[key]='simulation_default';}else{result[key]=args[key].trim();result.field_sources[key]='run_parameters';}
  } return result;
}
function selected(c,args) {
  const ids=Array.isArray(args.selected_ids)?args.selected_ids:String(args.selected_ids??'').split(',').filter(Boolean);
  const tags=String(args.tag_filter??'').split(',').filter(Boolean);
  return c.enabled!==false && (!ids.length||ids.includes(c.id)) && (!args.name_filter||c.name.includes(args.name_filter)) && (!tags.length||tags.some(t=>c.tags?.includes(t)));
}
export async function preflight(ctx) {
  const checks=[];
  try {
    const p=paths(ctx);
    if(fs.existsSync(p.manifest)) fs.unlinkSync(p.manifest);
    const args=ctx.args, station=ctx.fpgaStation??(ctx.configPath&&fs.existsSync(ctx.configPath)?read(ctx.configPath):{}), id=identity(args), a=adapter(ctx);
    if(station.cleanup_timeout_ms!==undefined&&(!Number.isFinite(station.cleanup_timeout_ms)||station.cleanup_timeout_ms<=0))throw fail('STATION.OPTIONS_INVALID','cleanup_timeout_ms must be positive');
    const loops=Number(args.loop_count??1), duration=Number(args.duration_limit_s??0);
    if(!Number.isInteger(loops)||loops<1||loops>100000||!Number.isFinite(duration)||duration<0||!['fresh','failed','unfinished'].includes(args.resume_mode??'fresh')) throw fail('RUN.OPTIONS_INVALID');
    const root=ctx.repositoryRoot??path.dirname(path.resolve(args.config_dir));
    const project=checkedProject(ctx.projectConfig,{projectId:ctx.project,testId:path.basename(p.test)});
    const manifest=discover(path.resolve(args.config_dir),root,project);
    if(args.execution_mode==='simulation') for(const key of ['board_revision','fpga_model','fixture_id','station_id']) if(!args[key]?.trim()) {const expected=[...new Set(manifest.items.filter(i=>selected(i.config,args)).map(i=>i.config.dut?.[key]).filter(Boolean))];if(expected.length===1){id[key]=expected[0];id.field_sources[key]='project_config.simulation_constraint';}}
    checks.push(...manifest.errors.map(e=>({...e,ok:false})));
    const devices=await a.devices();
    for(const i of manifest.items) {
      i.selected=ctx.testMatrix?ctx.testMatrix.selected_item_ids.includes(i.config.id):selected(i.config,args);
      if(!i.selected) continue;
      try {
        for(const key of ['board_revision','fpga_model','fixture_id','station_id']) if(i.config.dut?.[key] && i.config.dut[key]!==id[key]) throw fail('DUT.IDENTITY_MISMATCH',key);
        i.resources=resolveResources(i.config.instruments,devices);
        if(bool(args.ai_global_enable)||i.config.verdict.ai_assist?.required) validateAI(i.config.verdict.ai_assist,station);
        await a.preflight({config:i.config,resources:i.resources,identity:id,station});
      } catch(e) {checks.push({ok:false,item_id:i.config.id,code:e.code??e.message,message:e.message});}
    }
    if(!manifest.items.some(i=>i.selected)) checks.push({ok:false,code:'RUN.EMPTY_SELECTION'});
    const estimate={minimum_s:manifest.items.filter(i=>i.selected).reduce((n,i)=>n+Math.max(i.config.log_capture.enabled?i.config.log_capture.start_at_s+i.config.log_capture.duration_s:0,...i.config.waveform_captures.map(w=>w.at_s)),0)*loops,maximum_s:manifest.items.filter(i=>i.selected).reduce((n,i)=>n+i.config.timeout_s,0)*loops};
    await a.storage({minimum_free_bytes:station.minimum_free_bytes??1073741824,estimate,items:manifest.items,loops});
    const frozen={...manifest,project_config:projectMetadata(project),project_config_snapshot:project,host_manifest_sha256:ctx.testMatrix?.manifest_sha256,identity:id,options:structuredClone(args),station,estimate,created_at:utc()};
    frozen.content_sha256=contentHash(frozen);
    const prior=await previousRun(ctx,args,p);
    if(prior&&prior.value.content_sha256!==frozen.content_sha256) throw fail('RUN.RESUME_CONTENT_CHANGED','配置、身份、固件或限值已变化，请重新建立测试');
    if(prior) for(const item of frozen.items) {const old=prior.latest.get(item.config.id);item.selected=item.selected&&(args.resume_mode==='failed'?old?.product_verdict==='FAIL':prior.value.archive_complete===false||old?.execution_status!=='completed');}
    if(!frozen.items.some(i=>i.selected)) checks.push({ok:false,code:'RUN.EMPTY_SELECTION',message:'历史记录没有需要重试的测试项'});
    frozen.sha256=digest(frozen);
    const matrix={schema_version:'wiparse.test-matrix/1',revision:fs.existsSync(p.matrix)?read(p.matrix).revision:0,run_id:randomUUID(),preflight_revision:ctx.testMatrix?.manifest_sha256??frozen.sha256,active_item_id:null,items:manifest.items.map(i=>({id:i.config.id,name:i.config.name,order:i.config.order??0,tags:i.config.tags??[],selected:i.selected,status:i.selected?'pending':'SKIPPED',phase:'preflight',attempt:1,progress:0,elapsed_s:0,message:checks.filter(c=>c.item_id===i.config.id).map(c=>c.code).join(';').slice(0,256),config_sha256:i.config_sha256,firmware_sha256:i.firmware.sha256,result_ref:null}))};
    matrix.project_config=projectMetadata(project);publish(p,matrix);
    if(checks.length) return {ok:false,step:'preflight',checks};
    atomic(p.manifest,frozen); return {ok:true,step:'preflight',checks,manifest_sha256:frozen.sha256,test_matrix:ctx.testMatrix?{manifest_sha256:ctx.testMatrix.manifest_sha256,revision:ctx.testMatrix.manifest_sha256,item_count:ctx.testMatrix.selected_item_ids.length}:undefined,estimate,industrial_approved:false,simulation_only:args.execution_mode==='simulation'};
  } catch(e) {return {ok:false,step:'preflight',checks:[...checks,{ok:false,code:e.code??e.message,message:e.message}]};}
}
export async function stop(ctx) {const p=paths(ctx); atomic(p.stop,{requested_at:utc()}); return {ok:true,step:'stop'};}
export default async function run(ctx) {
  if(ctx.lifecycle==='preflight'||ctx.preflightOnly) return preflight(ctx);
  if(ctx.lifecycle==='stop') return stop(ctx);
  const p=paths(ctx), a=adapter(ctx), clock=ctx.fpgaClock??defaultClock;
  ctx._fpgaInputCache=new Map();
  if(!fs.existsSync(p.manifest)) throw fail('RUN.PREFLIGHT_REQUIRED');
  const frozen=read(p.manifest), args=frozen.options;
  const {sha256,...manifestBody}=frozen;
  if(digest(manifestBody)!==sha256) throw fail('RUN.MANIFEST_CORRUPT');
  assertInputs(frozen,ctx);
  if(digest(ctx.args)!==digest(args)) throw fail('RUN.PREFLIGHT_STALE','运行参数已变化');
  const runDir=ctx.layout?.run_dir;
  if(!runDir) throw fail('RUN.DIRECTORY_REQUIRED');
  if(fs.existsSync(path.join(runDir,'artifacts/docs/configuration-manifest.json')))throw fail('RUN.DIRECTORY_ALREADY_USED','请使用新的运行目录，历史证据不可覆盖');
  const runId=ctx.testMatrix?.run_id??ctx.runId??randomUUID(), docs=path.join(runDir,'artifacts','docs'), reports=path.join(runDir,'artifacts','reports');
  const previous=await previousRun(ctx,args,p);
  if(previous&&previous.value.content_sha256!==frozen.content_sha256) throw fail('RUN.RESUME_CONTENT_CHANGED');
  const matrix=read(p.matrix); matrix.run_id=runId;
  if(matrix.project_config?.sha256!==frozen.project_config.sha256)throw fail('RUN.PREFLIGHT_STALE','矩阵工程共享配置不匹配');
  const report=path.join(reports,'summary.md'),indexFile=path.join(reports,'attempt-index.jsonl');
  const controlFile=path.join(p.test,'fpga-control.json');
  let stateUnknown=false;
  const stopped=()=>fs.existsSync(p.stop), started=clock.now(), results=[];
  // The launcher owns clearing old Stop before accepting a new start. Every Stop
  // visible after that boundary belongs to startup/execution and must survive here.
  const lock=stopped()?null:await a.acquire({station_id:frozen.identity.station_id,run_id:runId,resources:frozen.items.filter(i=>i.selected).map(i=>i.resources)});
  if(!lock&&!stopped()) throw fail('RUN.STATION_BUSY');
  let cancelled=false, executionError=false, archivalError=false, halt=false,completedCount=0,hasFail=false,allPass=true,pausedMs=0;
  const totalExpected=frozen.items.filter(i=>i.selected).length*Number(args.loop_count??1);
  try {
    // Freeze recovery inputs before any report rendering can fail.
    atomic(path.join(docs,'configuration-manifest.json'),frozen);
    atomic(path.join(docs,'run-state.json'),{run_id:runId,pid:process.pid,status:'running',started_at:utc()});
    atomic(path.join(docs,'dut-identity.json'),frozen.identity);
    atomic(path.join(docs,'project-config.snapshot.json'),frozen.project_config_snapshot);
    atomic(path.join(docs,'limits.snapshot.json'),{mode:'engineering_editable',rules:frozen.items.map(i=>({id:i.config.id,rules:i.config.verdict.rules})),sha256:digest(frozen.items.map(i=>i.config.verdict.rules))});
    fs.mkdirSync(reports,{recursive:true});fs.writeFileSync(indexFile,'',{flag:'wx'});
    beginReport(report,frozen,runId);
    if(!fs.existsSync(controlFile)||read(controlFile).run_id!==runId)atomic(controlFile,{run_id:runId,state:'running'});
    for(let cycle=1;cycle<=Number(args.loop_count??1)&&!halt;cycle++) {
      matrix.cycle=cycle;
      for(const item of frozen.items.filter(i=>i.selected)) {
        pausedMs+=await pauseBoundary({file:controlFile,runId,clock,stopped,changed:state=>{publish(p,matrix);atomic(path.join(p.test,'status.json'),{step:state,hint:state==='paused'?'已暂停，可继续或停止':'继续测试',cycle,elapsed_s:(clock.now()-started-pausedMs)/1000});append(path.join(docs,'control-events.jsonl'),{state,at:utc()});}});
        if(stopped()) {cancelled=true;halt=true;break;}
        if(Number(args.duration_limit_s)>0 && clock.now()-started-pausedMs>=Number(args.duration_limit_s)*1000) {halt=true;break;}
        assertInputs(frozen,ctx);
        const old=previous?.latest.get(item.config.id);
        const attempt=randomUUID(), dir=path.join(runDir,'artifacts','items',item.config.id,attempt), c=item.config;
        const row=matrix.items.find(i=>i.id===c.id); Object.assign(row,{status:'running',phase:'reset',attempt:row.phase==='preflight'?1:(row.attempt??0)+1,attempt_id:attempt,loop_index:cycle,elapsed_s:0,progress:0,result_ref:null});matrix.active_item_id=c.id;publish(p,matrix);
        let evidence={}, verdict={product_verdict:'INCONCLUSIVE',results:[]}, result={schema_version:'wiparse.fpga-result/1',run_id:runId,attempt_id:attempt,previous_attempt_id:old?.attempt_id,item_id:c.id,cycle,identity:frozen.identity,config_sha256:item.config_sha256,firmware_sha256:item.firmware.sha256,limits_sha256:digest(c.verdict.rules),resolved_instruments:item.resources,suite_t0:started,item_t0:clock.now(),started_at:utc(),industrial_approved:false,validation_verdict:'NOT_APPROVED',execution_status:'completed',errors:[]};
        const io=guardedAdapter(a,{stopped,clock,deadline:result.item_t0+c.timeout_s*1000,cleanupTimeoutMs:frozen.station.cleanup_timeout_ms??10000,onUnknown:()=>{stateUnknown=true;halt=true;}});
        Object.assign(result,{attempt:row.attempt,loop_index:cycle,project_config:frozen.project_config,scope:{project_id:frozen.project_config_snapshot.project_id,test_id:frozen.project_config_snapshot.test_id,plugin_id:frozen.project_config_snapshot.plugin_id},config_sources:item.sources,effective_config_sha256:item.effective_config_sha256});
        result.schema_version='wiparse.fpga-result/2';
        result.config_sources={...item.sources,identity:structuredClone(frozen.identity.field_sources)};
        const heartbeat=setInterval(()=>{row.elapsed_s=(clock.now()-result.item_t0)/1000;matrix.elapsed_s=(clock.now()-started)/1000;try{publish(p,matrix);}catch{halt=true;executionError=true;archivalError=true;}},1000);
        try {
          atomic(path.join(dir,'config.snapshot.json'),c);atomic(path.join(dir,'raw-case.snapshot.json'),item.raw_config);atomic(path.join(dir,'firmware.json'),item.firmware);
          append(path.join(docs,'attempt-started.jsonl'),{item_id:c.id,attempt_id:attempt,execution_status:'running',started_at:utc()});
          const event=await resetDut(c.reset,item.resources,io,clock,stopped); result.capture_t0=event.capture_t0;append(path.join(docs,'reset-events.jsonl'),{...event,attempt_id:attempt});
          row.phase='capture';row.progress=0.3;publish(p,matrix);
          evidence=await capture(c,item.resources,io,clock,stopped,event,dir,frozen.station);
          const measurements=flattenRules(c.verdict.rules).filter(r=>r.type==='instrument_measure_range');
          if(measurements.length) {
            if(typeof a.measure!=='function') throw fail('HOST.MEASUREMENT_REQUIRED');
            evidence.measurements={};
            for(const rule of measurements) {if(stopped()) throw fail('RUN.ABORTED');evidence.measurements[rule.id]=await io.measure({rule,resources:item.resources,dir});}
          }
          verdict=await evaluateBounded(c.verdict.rules,evidence,{timeoutMs:Math.min(frozen.station.rule_timeout_ms??2000,c.timeout_s*1000-(clock.now()-result.item_t0)),stopped});
          if(bool(args.ai_global_enable)) result.ai=await advise(c.verdict.ai_assist,frozen.station,a,evidence,verdict);
          if(c.verdict.ai_assist?.enabled&&c.verdict.ai_assist.required&&result.ai?.status!=='completed'&&verdict.product_verdict!=='FAIL') verdict.product_verdict='INCONCLUSIVE';
          if(result.ai) atomic(path.join(dir,'ai','analysis.json'),result.ai);
        } catch(e) {
          evidence=e.evidence??evidence;
          if(!String(e.code).startsWith('RULE.')&&e.code!=='RUN.ABORTED')try{verdict=await evaluateBounded(c.verdict.rules,evidence,{timeoutMs:frozen.station.rule_timeout_ms??2000});}catch{}
          result.execution_status=e.code==='RUN.ABORTED'||stopped()?'cancelled':'error';result.errors.push({code:e.code??'EXECUTION.ERROR',message:e.message});
          cancelled||=result.execution_status==='cancelled';executionError||=result.execution_status==='error';
          if(['ENOSPC','EPERM','EACCES','EIO','EROFS'].includes(e.code)){archivalError=true;halt=true;}
          if(String(e.code).startsWith('IO.')||result.execution_status==='cancelled')halt=true;
        } finally {
          clearInterval(heartbeat);
          try {if(stateUnknown) throw fail('IO.STATE_UNKNOWN','设备动作未确认停止，保留租约等待人工恢复');result.cleanup=await io.cleanup({config:c,resources:item.resources,safe_power_state:frozen.station.safe_power_state??'off'});if(result.cleanup?.ok!==true)throw fail('CLEANUP.ERROR','Cleanup not confirmed');} catch(e) {result.errors.push({code:e.code??'CLEANUP.ERROR',message:e.message});result.execution_status='error';executionError=true;halt=true;}
        }
        if(stopped()) {cancelled=true;result.execution_status='cancelled';}
        if(result.execution_status!=='completed'&&verdict.product_verdict!=='FAIL')verdict.product_verdict='INCONCLUSIVE';
        Object.assign(result,verdict,{evidence,completed_at:utc(),elapsed_s:(clock.now()-result.item_t0)/1000,first_error_id:result.errors[0]?.code??null});
        hasFail||=verdict.product_verdict==='FAIL';
        if(result.evidence.log) {result.evidence.log.line_count=result.evidence.log.lines?.length;delete result.evidence.log.lines;delete result.evidence.log.text;}
        atomic(path.join(dir,'result.json'),result);append(path.join(docs,'attempt-events.jsonl'),{attempt_id:attempt,item_id:c.id,result_ref:path.relative(runDir,path.join(dir,'result.json')),execution_status:result.execution_status});
        const entry={item_id:c.id,name:c.name,attempt_id:attempt,previous_attempt_id:old?.attempt_id,cycle,started_at:result.started_at,completed_at:result.completed_at,execution_status:result.execution_status,product_verdict:result.product_verdict,result_ref:path.relative(reports,path.join(dir,'result.json')).split(path.sep).join('/')};
        entry.attempt=row.attempt;append(indexFile,entry);completedCount++;hasFail||=result.product_verdict==='FAIL';allPass&&=result.product_verdict==='PASS';
        results.push(entry);if(results.length>1000)results.shift();appendReport(report,result,dir,c.name);
        row.status=result.execution_status==='cancelled'?'ABORTED':result.execution_status==='error'?'ERROR':result.product_verdict;row.result_ref=path.relative(ctx.dataRoot&&ctx.project?path.join(ctx.dataRoot,'projects',ctx.project):p.test,path.join(dir,'result.json')).split(path.sep).join('/');row.elapsed_s=result.elapsed_s;row.phase='finished';row.progress=1;row.message=result.errors[0]?.code??'';matrix.active_item_id=null;publish(p,matrix);
        if(halt || cancelled || (bool(args.stop_on_fail)&&result.product_verdict==='FAIL')) {halt=true;break;}
      }
    }
  } catch(e) {if(e.code==='RUN.ABORTED')cancelled=true;else {executionError=true;archivalError=true;}for(const row of matrix.items)if(row.status.toLowerCase()==='running'){row.status=cancelled?'ABORTED':'ERROR';row.phase='archive_error';row.message=String(e.code??'RUN.ERROR').slice(0,256);}try{append(path.join(docs,'errors.jsonl'),{code:e.code??'RUN.ERROR',message:e.message});}catch{}}
  finally {
    if(lock&&!stateUnknown)try{await a.release(lock);}catch(e){executionError=true;try{append(path.join(docs,'errors.jsonl'),{code:'RELEASE.ERROR',message:e.message});}catch{}}
    matrix.active_item_id=null;try{publish(p,matrix);}catch{executionError=true;archivalError=true;}
    try{atomic(path.join(docs,'matrix-final.json'),matrix);}catch{executionError=true;archivalError=true;}
    try{atomic(path.join(docs,'run-state.json'),{run_id:runId,status:stateUnknown?'recovery_required':cancelled?'cancelled':executionError?'error':'finalizing',archive_complete:false,completed_at:utc(),lease_retained:stateUnknown});}catch{executionError=true;archivalError=true;}
  }
  const product=hasFail?'FAIL':!cancelled&&!executionError&&completedCount===totalExpected&&completedCount>0&&allPass?'PASS':'INCONCLUSIVE';
  const suite={simulation_only:args.execution_mode==='simulation',ok:!cancelled&&!executionError,execution_status:cancelled?'cancelled':executionError?'error':'completed',product_verdict:product,validation_verdict:'NOT_APPROVED',industrial_approved:false,run_id:runId,parent_run_id:previous?.value.run_id,content_sha256:frozen.content_sha256,manifest_sha256:frozen.sha256,identity:frozen.identity,attempt_count:completedCount,results_truncated:completedCount>results.length,attempt_index_ref:'attempt-index.jsonl',recovery_required:stateUnknown,results};
  suite.project_config=frozen.project_config;
  try{fs.appendFileSync(report,`\n## 套件结论\n\n产品结论：${product}；执行：${suite.execution_status}；完成 ${completedCount}/${totalExpected} 个attempt。\n`);}catch{executionError=true;archivalError=true;}
  const archiveError=()=>{suite.ok=false;suite.execution_status=cancelled?'cancelled':'error';if(suite.product_verdict!=='FAIL')suite.product_verdict='INCONCLUSIVE';};
  if(executionError)archiveError();
  suite.archive_complete=!archivalError;
  try{atomic(path.join(reports,'suite-result.json'),suite);}catch{executionError=true;archivalError=true;suite.archive_complete=false;archiveError();}
  try{atomic(path.join(docs,'run-state.json'),{run_id:runId,status:stateUnknown?'recovery_required':cancelled?'cancelled':executionError?'error':'completed',archive_complete:!archivalError,completed_at:utc(),lease_retained:stateUnknown});}catch{executionError=true;suite.archive_complete=false;archiveError();try{atomic(path.join(reports,'suite-result.json'),suite);}catch{}}
  if(archivalError||suite.archive_complete===false)try{fs.appendFileSync(report,'\n## 归档未完成\n\n本次运行归档失败，不能视为完成或通过；存储恢复后请重新预检并恢复运行。以 run-state.json 与 suite-result.json 的归档状态为准。\n');}catch{}
  return {...suite,summary_md:path.join(reports,'summary.md'),artifacts:{reports:[path.join(reports,'summary.md'),path.join(reports,'suite-result.json')]}};
}
