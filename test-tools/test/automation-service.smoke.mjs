// End-to-end SW smoke, no DUT, power outputs or firmware commands.
import assert from 'node:assert/strict';
import {mkdtemp,readFile,writeFile,mkdir,rm} from 'node:fs/promises';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {createHash} from 'node:crypto';
import {createServer} from 'node:net';
import os from 'node:os';
import path from 'node:path';
import {AutomationClient} from '../lib/automation-client.mjs';
const exe=path.resolve(process.argv[2]??'target/debug/wiparse-service.exe');
const root=await mkdtemp(path.join(os.tmpdir(),'wiparse-http-smoke-'));
const listener=createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));
const processHandle=spawn(exe,['--data-root',root,'--port',String(port)],{windowsHide:true,stdio:['ignore','pipe','pipe']});
let diagnostics='';processHandle.stderr.on('data',b=>{diagnostics+=b;});processHandle.stdout.on('data',()=>{});
let spawnError;processHandle.on('error',e=>{spawnError=e;});
const client=new AutomationClient(root);const started=Date.now();
try {
  while(true){if(spawnError)throw spawnError;if(processHandle.exitCode!==null)throw new Error(diagnostics);
    try{await client.request('/v1/health');break;}catch(e){if(Date.now()-started>10000)throw e;await new Promise(r=>setTimeout(r,50));}}
  assert.equal((await fetch(`http://127.0.0.1:${port}/v1/health`)).status,401);
  const caps=await client.invoke('system.capabilities');assert.equal(caps.industrial_approved,false);assert(caps.implemented.includes('protocol.decode'));
  await assert.rejects(client.invoke('instrument.output.control',{}),/UNSUPPORTED/);
  const lease=await client.invoke('device.lease.acquire',{resource:'MOCK_ONLY'});
  await assert.rejects(client.invoke('device.lease.acquire',{resource:'MOCK_ONLY'}),/BUSY/);
  await assert.rejects(client.invoke('device.lease.release',{...lease,session_epoch:'stale'}),/FENCED/);
  await client.invoke('device.lease.release',lease);
  const pairs=[0,1000,2000].map(n=>({tick:(9007199254740993n+BigInt(n)).toString(),reference_s:10+n/1000}));
  const mapping=await client.invoke('time.mapping.fit',{clock_id:'io',reference_clock_id:'scope',boot_id:'b',capture_session_id:'s',tick_hz:1000,sync_method:'hardware_edges',pairs,sync_uncertainty_s:0.001,max_residual_s:0.001,quality:{issues:[]}});
  assert.equal((await client.invoke('time.mapping.convert',{mapping,tick:'9007199254741493',boot_id:'b',capture_session_id:'s'})).reference_s,10.5);
  await assert.rejects(client.invoke('time.mapping.convert',{mapping,tick:'9007199254741493',boot_id:'new',capture_session_id:'s'}),/SESSION/);
  const dir=path.join(root,'projects/smoke/tests/offline/runs/1');await mkdir(dir,{recursive:true});
  const raw=Array.from({length:100},(_,i)=>`${i*0.000001},${i%10<5?0:3}`).join('\n');
  const wave=path.join(dir,'wave.csv');await writeFile(wave,raw);
  const params={command_id:'wave-once',path:wave,sha256:createHash('sha256').update(raw).digest('hex'),options:{threshold_v:1.5,burst_gap_s:0.00002}};
  const a=await client.invoke('wave.analyze',params);assert.equal((await client.invoke('wave.analyze',params)).job_id,a.job_id);
  await assert.rejects(client.invoke('wave.analyze',{...params,sha256:'different'}),/CONFLICT/);
  const job=await client.wait(a.job_id,{timeoutMs:10000});assert.equal(job.state,'succeeded',job.error);
  const result=JSON.parse(await readFile(job.result.path,'utf8'));assert.equal(result.industrial_approved,false);
  const frequency=result.measurements.find(m=>m.metric_id==='frequency_median');assert(Math.abs(frequency.value-100000)<0.001);
  const limits={limits_ref:'simulation',limits_hash:'not-an-industrial-approval',approved:true,unit:'Hz',minimum:99000,maximum:101000,require_uncertainty:false};
  assert.equal((await client.invoke('check.evaluate',{measurement:frequency,limits})).product_verdict,'PASS');
  assert.equal((await client.invoke('check.evaluate',{measurement:{...frequency,quality:{issues:['ACQ.OVERFLOW']}},limits})).product_verdict,'INCONCLUSIVE');
  assert.equal((await client.invoke('check.evaluate',{measurement:{...frequency,value:null},limits})).product_verdict,'INCONCLUSIVE');
  const denied=await client.invoke('wave.analyze',{...params,command_id:'outside',path:path.join(root,'jobs.json')});assert.equal((await client.wait(denied.job_id)).state,'failed');
  const uartBits=[1,1];for(let n=0;n<700;n++){uartBits.push(0);for(let b=0;b<8;b++)uartBits.push((0x55>>b)&1);uartBits.push(1);}uartBits.push(1,1);
  const uartRaw=uartBits.flatMap((bit,i)=>Array.from({length:8},(_,j)=>`${(i*8+j)/(115200*8)},${bit*3.3}`)).join('\n');
  const uartPath=path.join(dir,'uart.csv');await writeFile(uartPath,uartRaw);
  const decoded=await client.invoke('protocol.decode',{command_id:'protocol-batch',path:uartPath,sha256:createHash('sha256').update(uartRaw).digest('hex'),kind:'uart',channels:{signal:0},baud:115200,byte_budget:1024});
  const decodedJob=await client.wait(decoded.job_id);assert.equal(decodedJob.state,'succeeded',decodedJob.error);
  const protocol=JSON.parse(await readFile(decodedJob.result.path,'utf8'));assert.equal(protocol.complete,true);assert.equal(protocol.observed_frames,700);assert.equal(protocol.integrity_error_frames,0);
  assert.equal(createHash('sha256').update(await readFile(protocol.frames_path)).digest('hex'),protocol.frames_sha256);
  await client.request('/v1/stop',{});if(processHandle.exitCode===null)await once(processHandle,'exit');assert.equal(processHandle.exitCode,0,diagnostics);
  console.log(JSON.stringify({passed:true,version:(await readFile(path.join(root,'jobs.json'),'utf8')).length>0?'persistent_ledger_verified':'unknown',checks:['auth','capability_gate','lease_fencing','uint64_clock_mapping','idempotency','wave_frequency','quality_gate','null_gate','path_boundary','persistent_result','protocol_700_frames','frames_hash','graceful_stop']}));
} finally {
  if(!spawnError&&processHandle.exitCode===null){processHandle.kill();await once(processHandle,'exit').catch(()=>{});}
  // Only remove the fresh temporary directory returned by mkdtemp.
  if(path.dirname(root)===os.tmpdir()&&path.basename(root).startsWith('wiparse-http-smoke-'))await rm(root,{recursive:true,force:true});
}
