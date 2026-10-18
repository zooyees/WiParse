import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {performance} from 'node:perf_hooks';
import {hash} from '../lib/common.mjs';
import {guardedAdapter} from '../lib/io-guard.mjs';
import {REQUIRED_CAPABILITIES} from '../lib/host-adapter.mjs';
import {advise} from '../lib/ai-adapter.mjs';
import run,{preflight as pluginPreflight} from '../index.mjs';
import {prepareProjectFixture} from './fixtures/project-fixture.mjs';
const preflight=ctx=>{prepareProjectFixture(ctx);return pluginPreflight(ctx);};

function fixture(t){
 const root=fs.mkdtempSync(path.join(process.cwd(),'fpga-fault-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
 const cases=path.join(root,'cases');fs.mkdirSync(cases);fs.writeFileSync(path.join(root,'fw.bin'),'fw');
 const c={schema_version:'wiparse.fpga-test-case/1',id:'first',name:'First',order:1,enabled:true,dut:{},timeout_s:0.15,instruments:{programmer:{kind:'debug_probe',selector:{serial:'D1'},required:true},log:{kind:'serial_port',selector:{port:'COM1',baud:115200},required:true},scope:{kind:'oscilloscope',selector:{serial:'S1'},required:true}},firmware:{download:false,path:'../fw.bin',sha256:hash('fw'),target:'FPGA',format:'bin',base_address:0},reset:{method:'wiparse_dongle',dongle_role:'programmer',hardware:true,release_confirmation:{type:'command_complete',timeout_ms:1}},timing:{anchor:'after_reset_release'},log_capture:{enabled:true,start_at_s:0,duration_s:0.001,max_lines:100,encoding:'utf-8'},waveform_captures:[],verdict:{mode:'all',rules:[{id:'ok',type:'log_contains',required:true,text:'OK'}]}};
 const save=()=>{fs.writeFileSync(path.join(cases,'first.json'),JSON.stringify(c));fs.writeFileSync(path.join(cases,'second.json'),JSON.stringify({...c,id:'second',name:'Second',order:2}));};save();
 const a={version:'wiparse.fpga-host/1',capabilities:REQUIRED_CAPABILITIES,devices:async()=>[{kind:'debug_probe',serial:'D1'},{kind:'serial_port',port:'COM1'},{kind:'oscilloscope',serial:'S1'}],preflight:async()=>{},storage:async()=>{},acquire:async()=>({lease:1}),release:async()=>{},sourceOutput:async()=>{},voltage:async()=>12,probeReset:async()=>{},capture:async({dir})=>{const file=path.join(dir,'shot.png');fs.writeFileSync(file,'shot');return {screenshot:{path:file,sha256:hash('shot')}};},waveformSource:async()=>{},cleanup:async()=>({ok:true}),cancelAndConfirm:async()=>({quiescent:true}),collectLog:async()=>({raw_base64:Buffer.from('OK').toString('base64'),lines:[],start_cursor:0,end_cursor:2,dropped_lines:0,window_covered:true,complete:true})};
 const testDir=path.join(root,'test');
 const ctx={testDir,repositoryRoot:root,args:{config_dir:cases,resume_mode:'fresh',dut_serial:'SN',board_revision:'A',fpga_model:'FPGA',fixture_id:'FIX',station_id:'STATION'},layout:{run_dir:path.join(testDir,'runs','one')},fpgaAdapter:a,fpgaStation:{cleanup_timeout_ms:10},fpgaClock:{now:()=>0,sleep:async()=>{}}};
 return {root,c,cases,save,a,ctx};
}
const read=p=>JSON.parse(fs.readFileSync(p,'utf8'));
const sleep=ms=>new Promise(r=>setTimeout(r,ms));

for(const stage of ['before-start','launcher-lock-wait','acquire-wait','before-first-reset','null-acquire-stop'])test(`startup Stop at ${stage} survives initialization without reset/capture or lease leak`,async t=>{
 const {ctx,a,c,save}=fixture(t);c.timeout_s=4;save();const stopFile=path.join(ctx.testDir,'fpga.stop');
 let acquisitions=0,releases=0,resets=0,captures=0,entered,proceed;const acquired=new Promise(resolve=>{entered=resolve;}),gate=new Promise(resolve=>{proceed=resolve;});
 a.acquire=async()=>{acquisitions++;if(stage==='acquire-wait'){entered();await gate;}if(stage==='null-acquire-stop'){fs.writeFileSync(stopFile,'startup stop');return null;}return {lease:'only'};};
 a.release=async lease=>{assert.deepEqual(lease,{lease:'only'});releases++;};a.probeReset=async()=>{resets++;};a.collectLog=async()=>{captures++;throw Error('must not capture');};a.capture=async()=>{captures++;throw Error('must not capture');};
 assert.equal((await preflight(ctx)).ok,true);const rename=fs.renameSync;let running;
 try{
  if(stage==='before-start')fs.writeFileSync(stopFile,'startup stop');
  if(stage==='launcher-lock-wait'){
   // Model the launcher awaiting its configuration lock before entering the plugin.
   running=gate.then(()=>run(ctx));fs.writeFileSync(stopFile,'startup stop');proceed();
  }else{
   if(stage==='before-first-reset')fs.renameSync=(from,to)=>{const value=rename(from,to);if(String(to).endsWith('config.snapshot.json'))fs.writeFileSync(stopFile,'startup stop');return value;};
   running=run(ctx);
   if(stage==='acquire-wait'){await acquired;fs.writeFileSync(stopFile,'startup stop');proceed();}
  }
  const result=await running;assert.equal(result.execution_status,'cancelled');assert.equal(result.product_verdict,'INCONCLUSIVE');assert.equal(result.ok,false);assert.equal(resets,0);assert.equal(captures,0);
  assert.equal(acquisitions,['before-start','launcher-lock-wait'].includes(stage)?0:1);assert.equal(releases,['acquire-wait','before-first-reset'].includes(stage)?1:0);
  assert.equal(fs.readFileSync(stopFile,'utf8'),'startup stop');
  const state=read(path.join(ctx.layout.run_dir,'artifacts/docs/run-state.json'));assert.equal(state.status,'cancelled');assert.equal(state.lease_retained,false);
  assert(!read(path.join(ctx.testDir,'matrix.json')).items.some(row=>row.status==='running'));
 }finally{fs.renameSync=rename;}
});
test('residual Stop conservatively cancels; explicit launcher preparation permits a new run',async t=>{
 const {ctx,a,c,save}=fixture(t);c.timeout_s=4;save();let acquired=0,released=0,resets=0;
 a.acquire=async()=>{acquired++;return {lease:'one'};};a.release=async()=>released++;a.probeReset=async()=>resets++;
 assert.equal((await preflight(ctx)).ok,true);const stopFile=path.join(ctx.testDir,'fpga.stop');fs.writeFileSync(stopFile,'old stop');
 const old=await run(ctx);assert.equal(old.execution_status,'cancelled');assert.equal(acquired,0);assert.equal(resets,0);assert.equal(fs.readFileSync(stopFile,'utf8'),'old stop');
 // Only the external launcher, before accepting a new start, explicitly prepares it.
 fs.unlinkSync(stopFile);ctx.layout.run_dir=path.join(ctx.testDir,'runs','prepared');
 const next=await run(ctx);assert.equal(next.product_verdict,'PASS');assert.equal(resets,2);assert.equal(acquired,1);assert.equal(released,1);
});
test('Stop written after explicit new-start preparation is never treated as residual',async t=>{
 const {ctx,a,c,save}=fixture(t);c.timeout_s=4;save();let acquired=0,resets=0;a.acquire=async()=>{acquired++;return {};};a.probeReset=async()=>resets++;
 assert.equal((await preflight(ctx)).ok,true);const stopFile=path.join(ctx.testDir,'fpga.stop');fs.writeFileSync(stopFile,'old stop');fs.unlinkSync(stopFile);
 fs.writeFileSync(stopFile,'new accepted-start stop');const result=await run(ctx);
 assert.equal(result.execution_status,'cancelled');assert.equal(acquired,0);assert.equal(resets,0);assert.equal(fs.readFileSync(stopFile,'utf8'),'new accepted-start stop');
});

for(const method of ['probeReset','collectLog','capture','waveformSource','cleanup'])for(const quiescent of [true,false])for(const delayed of [false,true])test(`F02 ${method} ${delayed?'delayed':'direct'} reject, cancellation quiescent=${quiescent}`,async()=>{
 let cancelled=0,calls=0,unknown=false;
 const a={[method]:async()=>{if(delayed)await sleep(2);throw Object.assign(Error('transport rejection'),{code:'IO.TRANSPORT'});},capture:async()=>{calls++;return {};},cancelAndConfirm:async()=>{cancelled++;return {quiescent};}};
 // The failing method can itself be capture; keep the follow-up distinct.
 a.measure=async()=>{calls++;return {};};if(method==='capture')a.capture=async()=>{if(delayed)await sleep(2);throw Error('capture rejection');};
 const io=guardedAdapter(a,{stopped:()=>false,clock:{now:()=>0},deadline:1000,onUnknown:()=>unknown=true});
 await assert.rejects(io[method]({}),e=>e.code===(quiescent?'IO.TRANSPORT':'IO.STATE_UNKNOWN'));
 assert.equal(cancelled,1);assert.equal(unknown,!quiescent);
 if(!quiescent){await assert.rejects(io.measure({}),e=>e.code==='IO.STATE_UNKNOWN');assert.equal(calls,0);}
});
test('F02 explicitly quiescent business rejection permits ordinary cleanup without cancellation',async()=>{
 let cancelled=0,cleaned=0;const business=Object.assign(Error('device declined request'),{code:'DEVICE.BUSY',quiescent:true});
 const io=guardedAdapter({probeReset:async()=>{throw business;},cleanup:async()=>{cleaned++;return {ok:true};},cancelAndConfirm:async()=>{cancelled++;return {quiescent:false};}},{clock:{now:()=>0},deadline:1000,stopped:()=>false});
 await assert.rejects(io.probeReset({}),e=>e===business);await io.cleanup({});assert.equal(cancelled,0);assert.equal(cleaned,1);
});
for(const quiescent of [true,false])test(`F02 rejection confirmation blocks new actions while pending, quiescent=${quiescent}`,async()=>{
 let started,finish,calls=0;const barrier=new Promise(resolve=>{started=resolve;});
 const io=guardedAdapter({probeReset:()=>{throw Error('sync transport failure');},capture:async()=>{calls++;},cancelAndConfirm:()=>{started();return new Promise(resolve=>{finish=resolve;});}},{clock:{now:()=>0},deadline:1000,stopped:()=>false});
 const failed=assert.rejects(io.probeReset({}),e=>e.code===(quiescent?'IO.TRANSPORT':'IO.STATE_UNKNOWN'));
 await barrier;await assert.rejects(io.capture({}),e=>e.code==='IO.CANCELLING');assert.equal(calls,0);finish({quiescent});await failed;
});
test('F02 cancellation rejection leaves unknown state and forbids cleanup',async()=>{
 let cleaned=0,unknown=0;const io=guardedAdapter({probeReset:()=>Promise.reject(Error('transport')),cleanup:async()=>{cleaned++;},cancelAndConfirm:async()=>{throw Error('cancel failed');}},{clock:{now:()=>0},deadline:1000,stopped:()=>false,onUnknown:()=>unknown++});
 await assert.rejects(io.probeReset({}),e=>e.code==='IO.STATE_UNKNOWN');await assert.rejects(io.cleanup({}),e=>e.code==='IO.STATE_UNKNOWN');assert.equal(cleaned,0);assert.equal(unknown,1);
});
for(const failing of ['collectLog','capture','cleanup'])for(const quiescent of [true,false])test(`F02/F05 real run ${failing} reject, quiescent=${quiescent}: cancel siblings before release`,async t=>{
 const {ctx,a,c,save}=fixture(t);c.timeout_s=10;let resets=0,cleaned=0,released=0,pendingStarted,barrier=new Promise(resolve=>{pendingStarted=resolve;}),rejected=0;
 const cancellations=[];a.probeReset=async()=>{resets++;};a.release=async()=>{released++;};a.cleanup=async()=>{cleaned++;if(failing==='cleanup')throw Error('cleanup transport rejection');return {ok:true};};
 if(failing!=='cleanup'){
  c.waveform_captures=[{id:'w',at_s:0,timeout_s:5,kind:'screenshot',channels:['CH1'],source_waveform:{enabled:false}}];
  const sibling=failing==='collectLog'?'capture':'collectLog';
  a[sibling]=()=>{pendingStarted();return new Promise(()=>{});};
  a[failing]=async()=>{await barrier;rejected++;throw Error('direct transport rejection');};
 }else{const log=a.collectLog;a.collectLog=async()=>{const value=await log();return {...value,raw_base64:Buffer.from('bad').toString('base64')};};}
 a.cancelAndConfirm=async({method,operation_id})=>{cancellations.push({method,operation_id});return {quiescent};};save();
 assert.equal((await preflight(ctx)).ok,true);const result=await run(ctx);
 assert.equal(result.execution_status,'error');assert.equal(result.product_verdict,failing==='cleanup'?'FAIL':'INCONCLUSIVE');assert.equal(result.recovery_required,!quiescent);
 assert.equal(resets,1);assert.equal(released,quiescent?1:0);assert.equal(cleaned,failing==='cleanup'||quiescent?1:0);
 assert.deepEqual(cancellations.map(c=>c.method).sort(),(failing==='cleanup'?['cleanup']:['capture','collectLog']).sort());
 assert.equal(new Set(cancellations.map(c=>c.operation_id)).size,cancellations.length);
 const matrix=read(path.join(ctx.testDir,'matrix.json'));assert.equal(matrix.items[1].status,'pending');assert.equal(read(path.join(ctx.layout.run_dir,'artifacts/docs/run-state.json')).lease_retained,!quiescent);
 if(failing==='cleanup'){const item=read(path.resolve(ctx.layout.run_dir,'artifacts/reports',result.results[0].result_ref));assert.equal(item.product_verdict,'FAIL');assert(fs.existsSync(path.join(path.dirname(path.resolve(ctx.layout.run_dir,'artifacts/reports',result.results[0].result_ref)),'logs/serial.log')));}
 else assert.equal(rejected,1);
});
for(const quiescent of [true,false])test(`F02 scheduler cancels pending sibling after quiescent business error, sibling quiescent=${quiescent}`,async t=>{
 const {ctx,a,c,save}=fixture(t);c.timeout_s=10;c.waveform_captures=[{id:'w',at_s:0,timeout_s:5,kind:'screenshot',channels:['CH1'],source_waveform:{enabled:false}}];save();
 let started,barrier=new Promise(resolve=>{started=resolve;}),cleaned=0,released=0;const cancelled=[];
 a.capture=()=>{started();return new Promise(()=>{});};a.collectLog=async()=>{await barrier;throw Object.assign(Error('serial protocol declined'),{code:'SERIAL.DECLINED',quiescent:true});};
 a.cancelAndConfirm=async({method})=>{cancelled.push(method);return {quiescent};};a.cleanup=async()=>{cleaned++;return {ok:true};};a.release=async()=>{released++;};
 assert.equal((await preflight(ctx)).ok,true);const result=await run(ctx);assert.notEqual(result.product_verdict,'PASS');assert.deepEqual(cancelled,['capture']);assert.equal(result.recovery_required,!quiescent);assert.equal(cleaned,quiescent?1:0);assert.equal(released,quiescent?1:0);
});

for(const method of ['probeReset','collectLog','waveformSource','cleanup'])for(const scenario of ['deadline','stop','late','unknown']){
 test(`F02 ${method}: ${scenario} cannot become a late success`,async()=>{
  let stopped=false,unknown=false,calls=0,cancelled=0,finish;
  const a={[method]:()=>{calls++;if(scenario==='stop')stopped=true;return new Promise(resolve=>{finish=resolve;});},cancelAndConfirm:async()=>{cancelled++;if(scenario==='late'){finish({ok:true});await sleep(10);}return {quiescent:scenario!=='unknown'};}};
  const clock={now:()=>performance.now()};
  const io=guardedAdapter(a,{clock,stopped:()=>stopped,deadline:clock.now()+8,cleanupTimeoutMs:8,onUnknown:()=>unknown=true});
  const expected=scenario==='unknown'?'IO.STATE_UNKNOWN':scenario==='stop'&&method!=='cleanup'?'RUN.ABORTED':'IO.TIMEOUT';
  await assert.rejects(io[method]({}),e=>e.code===expected);
  assert.equal(cancelled,1);assert.equal(unknown,scenario==='unknown');
  if(unknown){await assert.rejects(io[method]({}),e=>e.code==='IO.STATE_UNKNOWN');assert.equal(calls,1);}
 });
}
for(const method of ['probeReset','collectLog','waveformSource','cleanup'])for(const quiescent of [true,false])for(const action of ['timeout','stop']){
 test(`F02 run ${method} ${action}, quiescent=${quiescent}, blocks next item`,async t=>{
  const {ctx,a,c,save}=fixture(t);let resets=0,releases=0,cancels=0;
  const reset=a.probeReset;a.probeReset=async(...args)=>{resets++;return reset(...args);};
  if(method==='waveformSource'){c.waveform_captures=[{id:'w',at_s:0,timeout_s:0.02,kind:'screenshot',channels:['CH1'],source_waveform:{enabled:true,format:'csv'}}];save();}
  const original=a[method];a[method]=(...args)=>{if(method==='probeReset')resets++;if(action==='stop')fs.writeFileSync(path.join(ctx.testDir,'fpga.stop'),'stop');return new Promise(()=>{});};
  a.cancelAndConfirm=async()=>{cancels++;return {quiescent};};a.release=async()=>releases++;
  assert.equal((await preflight(ctx)).ok,true);
  const result=await run(ctx);assert.equal(result.execution_status,action==='stop'?'cancelled':'error');assert.notEqual(result.product_verdict,'PASS');
  assert.equal(resets,1);assert.equal(releases,quiescent?1:0);assert.equal(result.recovery_required,!quiescent);assert.equal(cancels,1);
  const state=read(path.join(ctx.layout.run_dir,'artifacts/docs/run-state.json'));
  assert.equal(state.status,quiescent?(action==='stop'?'cancelled':'error'):'recovery_required');assert.equal(state.lease_retained,!quiescent);
  const matrix=read(path.join(ctx.testDir,'matrix.json'));assert.equal(matrix.items[1].status,'pending');assert(!matrix.items.some(r=>r.status==='running'));
  a[method]=original;
  if(!quiescent){ctx.args.resume_mode='unfinished';ctx.args.resume_from=ctx.layout.run_dir;ctx.layout.run_dir=path.join(ctx.testDir,'runs','blocked');const pf=await preflight(ctx);assert.equal(pf.ok,false);assert(pf.checks.some(c=>c.code==='RUN.RECOVERY_REQUIRED'));assert.equal(resets,1);}
 });
}

for(const location of ['result','journal-start','journal-complete','index','partial-start','partial-complete','partial-index','matrix','matrix-terminal','matrix-final','report-item','report-final','suite','state-final'])for(const code of ['ENOSPC','EPERM']){
 test(`F08 ${location} ${code}: no suite PASS, journal supports recovery`,async t=>{
  const {ctx,a,c,save}=fixture(t);c.timeout_s=4;save();
  assert.equal((await preflight(ctx)).ok,true);
  const original={rename:fs.renameSync,open:fs.openSync,append:fs.appendFileSync,write:fs.writeSync};let injected=0,cleaned=false;const opened=new Map();
  a.cleanup=async()=>{cleaned=true;return {ok:true};};
  const fault=()=>{injected++;throw Object.assign(Error(`injected ${location}`),{code});};
  fs.renameSync=(from,to)=>{
   const p=String(to).replaceAll('\\','/');
   if((location==='result'&&p.endsWith('/result.json'))||(['matrix','matrix-terminal'].includes(location)&&(location==='matrix'||cleaned)&&p===path.join(ctx.testDir,'matrix.json').replaceAll('\\','/'))||(location==='matrix-final'&&p.endsWith('/matrix-final.json'))||(location==='suite'&&p.endsWith('/suite-result.json'))||(location==='state-final'&&p.endsWith('/run-state.json')&&read(from).status==='completed'))fault();
   return original.rename(from,to);
  };
  fs.openSync=(file,flags,...args)=>{
   const p=String(file).replaceAll('\\','/');
   if((location==='journal-start'&&p.endsWith('/attempt-started.jsonl'))||(location==='journal-complete'&&p.endsWith('/attempt-events.jsonl'))||(location==='index'&&flags==='a'&&p.endsWith('/attempt-index.jsonl')))fault();
   const fd=original.open(file,flags,...args);opened.set(fd,p);return fd;
  };
  fs.writeSync=(fd,value,...args)=>{
   const p=opened.get(fd)||'';
   if(typeof value==='string'&&((location==='partial-start'&&p.endsWith('/attempt-started.jsonl'))||(location==='partial-complete'&&p.endsWith('/attempt-events.jsonl'))||(location==='partial-index'&&p.endsWith('/attempt-index.jsonl')))){
    original.write(fd,value.slice(0,Math.floor(value.length/2)));fault();
   }
   return original.write(fd,value,...args);
  };
  fs.appendFileSync=(file,value,...args)=>{if(String(file).endsWith('summary.md')&&((location==='report-final'&&String(value).includes('## 套件结论'))||(location==='report-item'&&!String(value).includes('## 套件结论'))))fault();return original.append(file,value,...args);};
  let result;try{result=await run(ctx);}finally{fs.renameSync=original.rename;fs.openSync=original.open;fs.appendFileSync=original.append;fs.writeSync=original.write;}
  assert(injected>0);assert.equal(result.execution_status,'error');assert.notEqual(result.product_verdict,'PASS');
  const runState=read(path.join(ctx.layout.run_dir,'artifacts/docs/run-state.json'));assert(['error','finalizing'].includes(runState.status));assert.equal(runState.archive_complete,false);
  const finalFile=path.join(ctx.layout.run_dir,'artifacts/docs/matrix-final.json');
  if(fs.existsSync(finalFile))assert(!read(finalFile).items.some(r=>r.status==='running'));
  const suiteFile=path.join(ctx.layout.run_dir,'artifacts/reports/suite-result.json');if(fs.existsSync(suiteFile))assert.notEqual(read(suiteFile).product_verdict,'PASS');
  if(location==='suite'||location==='state-final')assert.match(fs.readFileSync(path.join(ctx.layout.run_dir,'artifacts/reports/summary.md'),'utf8'),/归档未完成/);
  const oldDir=ctx.layout.run_dir;
  ctx.args.resume_mode='unfinished';ctx.args.resume_from=oldDir;ctx.layout.run_dir=path.join(ctx.testDir,'runs','recovered');
  const pf=await preflight(ctx);assert.equal(pf.ok,true,JSON.stringify(pf));const recovery=await run(ctx);assert.equal(recovery.product_verdict,'PASS');
  assert(fs.existsSync(path.join(oldDir,'artifacts/docs/configuration-manifest.json')));
 });
}

for(const scenario of ['timeout','throw','malformed','empty-allowlist','local-empty-allowlist','disjoint-inputs'])test(`F12 AI ${scenario} respects response and payload boundaries`,async()=>{
 let payload;
 const response=await advise({enabled:true,provider_profile:'p',timeout_s:0.005,inputs:scenario==='disjoint-inputs'?['log']:['rule_results']},{allow_public_ai:true,ai_profiles:{p:{kind:scenario==='local-empty-allowlist'?'local':'public',model:'m',allow_fields:scenario.endsWith('empty-allowlist')?[]:['rule_results','waveform_metrics']}}},{ai:async value=>{payload=value.payload;if(scenario==='timeout')return new Promise(()=>{});if(scenario==='throw')throw Error('transport failure');if(scenario==='malformed')return {recommendation:'PASS',confidence:2};return {recommendation:'REVIEW',confidence:1};}},{log:{text:'private'},metrics:{supply:12}},{results:[{id:'r',verdict:'PASS',measured:1,unit:'Hz',private:'omit'}]});
 assert.equal(response.status,['timeout','throw','malformed'].includes(scenario)?'error':'completed');
 if(scenario.endsWith('empty-allowlist')||scenario==='disjoint-inputs')assert.deepEqual(payload,{});
 else assert.deepEqual(payload,{rule_results:[{id:'r',verdict:'PASS',measured:1,unit:'Hz'}]});
 assert.deepEqual(response.sent_fields,Object.keys(payload));assert(!JSON.stringify(response.redacted_input).includes('private'));
});
test('F08 recovery rejects corruption in a completed journal record',async t=>{
 const {ctx,c,save}=fixture(t);c.timeout_s=4;save();assert.equal((await preflight(ctx)).ok,true);await run(ctx);
 fs.appendFileSync(path.join(ctx.layout.run_dir,'artifacts/docs/attempt-started.jsonl'),'{bad complete record}\n');
 ctx.args.resume_mode='unfinished';ctx.args.resume_from=ctx.layout.run_dir;ctx.layout.run_dir=path.join(ctx.testDir,'runs','invalid');
 assert.equal((await preflight(ctx)).ok,false);
});
for(const scenario of ['timeout','throw','malformed'])for(const knownFail of [false,true])test(`F12 required AI ${scenario}, known FAIL=${knownFail}`,async t=>{
 const {ctx,a,c,save}=fixture(t);c.timeout_s=4;c.verdict.ai_assist={enabled:true,required:true,policy:'advisory_only',provider_profile:'p',timeout_s:0.005,inputs:['rule_results']};if(knownFail)c.verdict.rules[0].text='absent';save();
 ctx.args.ai_global_enable=true;ctx.fpgaStation.ai_profiles={p:{kind:'local',adapter:'test',model:'fixture',allow_fields:[]}};
 a.ai=async()=>{if(scenario==='timeout')return new Promise(()=>{});if(scenario==='throw')throw Error('AI transport error');return {recommendation:'PASS',confidence:-1};};
 assert.equal((await preflight(ctx)).ok,true);const result=await run(ctx);assert.equal(result.product_verdict,knownFail?'FAIL':'INCONCLUSIVE');
 for(const entry of result.results){const item=read(path.resolve(ctx.layout.run_dir,'artifacts/reports',entry.result_ref));assert.equal(item.ai.status,'error');assert.deepEqual(item.ai.sent_fields,[]);}
});
