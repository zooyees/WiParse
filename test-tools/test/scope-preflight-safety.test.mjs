import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import http from 'node:http';
import {acquireMonitorLock,awaitInstrumentJob,ensureNoActiveTest,probeWritableDirectory,readTargetLines,runLoop,selectSourceFile,targetSerialSession} from '../plugins/scope-serial-monitor/loop.mjs';

test('scope writable-directory probe preserves an existing user file',()=>{
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),'wiparse-probe-'));
  const existing=path.join(dir,'.write_probe');
  try {
    fs.writeFileSync(existing,'user evidence');
    probeWritableDirectory(dir);
    assert.equal(fs.readFileSync(existing,'utf8'),'user evidence');
    assert.deepEqual(fs.readdirSync(dir),['.write_probe']);
  } finally {fs.unlinkSync(existing);fs.rmdirSync(dir);}
});

test('monitor lock acquisition is exclusive and never replaces a prior lock',()=>{
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),'wiparse-scope-lock-'));const file=path.join(dir,'run.lock');
  try {
    const token=acquireMonitorLock(file,{pid:1});const original=fs.readFileSync(file,'utf8');
    assert.equal(JSON.parse(original).token,token);
    assert.throws(()=>acquireMonitorLock(file,{pid:2}),/监控锁已存在/);
    assert.equal(fs.readFileSync(file,'utf8'),original);
  } finally {fs.unlinkSync(file);fs.rmdirSync(dir);}
});

test('instrument submission is not treated as completion and failures stay failures',async()=>{
  const accepted={ok:true,data:{job_id:7,device_id:3}};let calls=0;
  const invoke=async(method,params)=>{assert.equal(method,'instrument.job.get');assert.equal(params.job_id,7);calls++;return {ok:true,data:{job_id:7,device_id:3,state:'completed',ok:true}};};
  assert.equal((await awaitInstrumentJob(invoke,3,accepted,1000)).ok,true);assert.equal(calls,1);
  await assert.rejects(awaitInstrumentJob(async()=>({ok:true,data:{job_id:7,device_id:3,state:'completed',ok:false,detail:{error:'SCPI failed'}}}),3,accepted,1000),error=>error.code==='INSTRUMENT.FAILED');
  await assert.rejects(awaitInstrumentJob(async()=>({ok:true,data:{job_id:7,state:'pending_or_unknown'}}),3,accepted,1),error=>error.code==='INSTRUMENT.UNKNOWN');
  await assert.rejects(awaitInstrumentJob(invoke,3,{ok:true,data:{}},1000),error=>error.code==='INSTRUMENT.UNKNOWN');
});

test('waveform evidence must be an actual nonempty file inside the output directory',()=>{
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'wiparse-source-'));const dir=path.join(root,'waves');fs.mkdirSync(dir);
  const outside=path.join(root,'outside.isf'),empty=path.join(dir,'empty.isf'),wave=path.join(dir,'capture.wfm');
  try {
    fs.writeFileSync(outside,'outside');fs.writeFileSync(empty,'');fs.writeFileSync(wave,'wave data');
    assert.equal(selectSourceFile(dir,[outside,empty,path.join(dir,'missing.isf')]),null);
    assert.equal(selectSourceFile(dir,[outside,wave]),fs.realpathSync(wave));
  } finally {fs.unlinkSync(outside);fs.unlinkSync(empty);fs.unlinkSync(wave);fs.rmdirSync(dir);fs.rmdirSync(root);}
});

test('monitor resolves the configured port and does not confuse another open session',()=>{
  const status={ok:true,data:{sessions:[{port:'COM3',tab_id:0,confirmed:true,baud:115200},{port:'COM8',tab_id:3,confirmed:true,baud:9600}]}};
  assert.equal(targetSerialSession(status,'com8').tab_id,3);
  assert.equal(targetSerialSession(status,'COM9'),null);
  assert.throws(()=>targetSerialSession({ok:false},'COM3'),/无法确认/);
});

test('log polling binds the configured port and rejects malformed responses',async()=>{
  let received;
  const read=async(method,params)=>{received={method,params};return {ok:true,data:{lines:['message'],total:11,next_row:11,dropped:2}};};
  const result=await readTargetLines(read,'COM8',10,200);
  assert.equal(received.method,'log.lines.get');assert.equal(received.params.port,'COM8');assert.equal(received.params.from_row,10);
  assert.deepEqual(result,{lines:['message'],total:11,next:11,dropped:2});
  await assert.rejects(readTargetLines(async()=>({ok:false}),'COM8',0),/无法读取/);
});

test('monitor refuses an active or unknown test instead of aborting it',async()=>{
  const methods=[];
  const active=async(method)=>{methods.push(method);return {ok:true,data:{running:true}};};
  await assert.rejects(ensureNoActiveTest(active),/其他测试正在运行/);
  assert.deepEqual(methods,['test.status']);
  await assert.rejects(ensureNoActiveTest(async()=>({ok:false})),/无法确认/);
  await ensureNoActiveTest(async()=>({ok:true,data:{running:false}}));
});

test('scope preflight checks a simulated service without interrupting its active test',async()=>{
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),'wiparse-scope-preflight-'));
  const protectedFile=path.join(dir,'.write_probe');fs.writeFileSync(protectedFile,'existing');
  const methods=[];let testRunning=true,jobApiAvailable=true;
  const server=http.createServer(async(req,res)=>{
    res.setHeader('content-type','application/json');
    if(req.url==='/v1/health'){res.end(JSON.stringify({ok:true,data:{version:'1.2.2'}}));return;}
    let body='';for await(const part of req)body+=part;
    const {method}=JSON.parse(body);methods.push(method);
    if(method==='instrument.job.get'&&!jobApiAvailable){res.end(JSON.stringify({ok:false,error:'unsupported'}));return;}
    const data=method==='test.status'?{running:testRunning}:method==='serial.monitor.status'?{sessions:[]}:method==='instrument.job.get'?{state:'pending_or_unknown',job_id:0}:method==='instrument.list'?{devices:[{device_id:1,kind:'oscilloscope',resource:'SIM',identity:{model:'SIM'}}]}:{};
    res.end(JSON.stringify({ok:true,data}));
  });
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  try {
    const cfg=JSON.parse(fs.readFileSync(new URL('../plugins/scope-serial-monitor/station.json',import.meta.url),'utf8'));
    cfg.gui.api=`http://127.0.0.1:${server.address().port}`;cfg.pdf.browsers=[process.execPath];
    cfg.paths={...cfg.paths,test_dir:dir,isf_dir:dir,report_dir:dir,shot_dir:dir,docs_dir:dir,lock_file:path.join(dir,'run.lock'),stop_file:path.join(dir,'run.stop'),status_file:path.join(dir,'status.json')};
    const result=await runLoop(cfg,{preflightOnly:true,pinTriggers:true,log:()=>{}});
    assert.equal(result.ok,false);assert(result.checks.some(item=>item.id==='test_idle'&&!item.ok));
    assert(!methods.includes('test.abort'));assert(!methods.includes('instrument.command'));
    assert.equal(fs.readFileSync(protectedFile,'utf8'),'existing');
    testRunning=false;
    jobApiAvailable=false;
    const unsupported=await runLoop(cfg,{preflightOnly:true,pinTriggers:true,log:()=>{}});
    assert.equal(unsupported.ok,false);assert(unsupported.checks.some(item=>item.id==='instrument_job_api'&&!item.ok));
    jobApiAvailable=true;
    const ready=await runLoop(cfg,{preflightOnly:true,pinTriggers:true,log:()=>{}});
    assert.equal(ready.ok,true);assert(!methods.includes('test.abort'));assert(!methods.includes('instrument.command'));
  } finally {
    await new Promise(resolve=>server.close(resolve));
    for(const name of fs.readdirSync(dir))fs.unlinkSync(path.join(dir,name));
    fs.rmdirSync(dir);
  }
});

test('simulated capture uses the selected serial port and only reports completed evidence',async()=>{
  const dir=fs.mkdtempSync(path.join(os.tmpdir(),'wiparse-scope-loop-'));
  const stop=path.join(dir,'run.stop');let serialOn=false,job=0,lineReads=0;const jobs=new Map(),calls=[];
  const server=http.createServer(async(req,res)=>{
    res.setHeader('content-type','application/json');
    if(req.url==='/v1/health'){res.end(JSON.stringify({ok:true,data:{version:'1.2.2'}}));return;}
    let body='';for await(const part of req)body+=part;const {method,params}=JSON.parse(body);calls.push({method,params});let data={};
    if(method==='test.status')data={running:false};
    else if(method==='ui.instrument.select')data={};
    else if(method==='instrument.list')data={devices:[{device_id:1,kind:'oscilloscope',resource:'SIM',identity:{model:'SIM'}}]};
    else if(method==='serial.status')data={monitoring:serialOn,port:'COM7'};
    else if(method==='serial.monitor.status')data={sessions:[{port:'COM3',tab_id:0,confirmed:true,baud:115200},...(serialOn?[{port:'COM7',tab_id:2,confirmed:true,baud:2000000}]:[])]};
    else if(method==='serial.monitor.start'){assert.equal(params.port,'COM7');serialOn=true;data={port:'COM7',tab_id:2};}
    else if(method==='serial.monitor.stop'){assert.equal(params.port,'COM7');serialOn=false;}
    else if(method==='log.lines.get'){assert.equal(params.port,'COM7');lineReads++;data=lineReads===1?{lines:[],total:0,next_row:0,dropped:0}:{lines:['timeout'],total:1,next_row:1,dropped:0};}
    else if(method==='instrument.command'){job++;jobs.set(job,{state:'completed',job_id:job,device_id:1,ok:true,detail:{}});data={job_id:job,device_id:1};}
    else if(method==='instrument.capture'){fs.writeFileSync(params.path,Buffer.alloc(64,1));job++;jobs.set(job,{state:'completed',job_id:job,device_id:1,ok:true,detail:{}});data={job_id:job,device_id:1};}
    else if(method==='instrument.waveform_source'){const file=path.join(dir,'actual-source.wfm');fs.writeFileSync(file,'real simulated waveform bytes');job++;jobs.set(job,{state:'completed',job_id:job,device_id:1,ok:true,detail:{files:[file]}});data={job_id:job,device_id:1};}
    else if(method==='instrument.job.get'){data=jobs.get(params.job_id)||{state:'pending_or_unknown',job_id:params.job_id};if(data?.detail?.files)fs.writeFileSync(stop,'stop after capture');}
    else {res.end(JSON.stringify({ok:false,error:`unexpected ${method}`}));return;}
    res.end(JSON.stringify({ok:true,data}));
  });
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));
  try {
    const cfg=JSON.parse(fs.readFileSync(new URL('../plugins/scope-serial-monitor/station.json',import.meta.url),'utf8'));
    cfg.gui.api=`http://127.0.0.1:${server.address().port}`;cfg.pdf.browsers=[process.execPath];
    Object.assign(cfg.timing,{post_trigger_delay_s:0,scope_settle_s:0,poll_ms:10,serial_stop_timeout_ms:300,shot_timeout_ms:1000,isf_timeout_ms:1000});
    cfg.paths={...cfg.paths,test_dir:dir,isf_dir:dir,report_dir:dir,shot_dir:dir,docs_dir:dir,lock_file:path.join(dir,'run.lock'),stop_file:stop,status_file:path.join(dir,'status.json')};
    const result=await runLoop(cfg,{pinTriggers:true,log:()=>{}});
    assert.equal(result.ok,false);assert.match(result.error,/报告生成失败/);assert(!fs.existsSync(path.join(dir,'run.lock')));
    assert(calls.some(call=>call.method==='instrument.waveform_source'));assert(calls.some(call=>call.method==='instrument.job.get'));
    assert(!calls.some(call=>call.method==='test.abort'));assert(calls.filter(call=>call.method==='log.lines.get').every(call=>call.params.port==='COM7'));
    assert.match(fs.readFileSync(result.summary_md,'utf8'),/actual-source\.wfm/);
  } finally {
    await new Promise(resolve=>server.close(resolve));
    for(const name of fs.readdirSync(dir))fs.unlinkSync(path.join(dir,name));fs.rmdirSync(dir);
  }
});
