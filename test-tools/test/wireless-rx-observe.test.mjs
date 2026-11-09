import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
const fromRepo=(relative)=>fileURLToPath(new URL('../../'+relative,import.meta.url));
import http from 'node:http';
import {spawn} from 'node:child_process';
import {execute} from '../hub-workspace.mjs';
import {createFpgaHostAdapter} from '../lib/host/fpga-host-adapter.mjs';
import run,{preflight,stop} from '../plugins/wireless-whitebox/index.mjs';

test('RX preflight refuses a simulated FT4222 in a hardware workflow',async()=>{
  const item=JSON.parse(fs.readFileSync(fromRepo('recipes/wireless-whitebox/cases/wcv_07.json')));
  const ctx={args:{mode:'rx_read'},projectConfig:{value:{station:{station_id:'lab',rx_i2c_interface:'bridge://ft4222/serial=DEMO',log_source:'COM7'}}},testMatrix:{values:[item],selected_item_ids:['wcv_07'],manifest_sha256:'x'},invoke:async method=>method==='instrument.list'?{data:{devices:[{kind:'usb_bridge',resource:'bridge://ft4222/serial=DEMO',device_id:3,identity:{firmware:'demo'}}]}}:{data:{ports:[{device:'COM7'}]}}};
  const result=await preflight(ctx);assert.equal(result.ok,false);assert.match(result.error,/DEMO_DEVICE_NOT_ALLOWED/);
});

test('MT5788 read mode executes leased FT4222 jobs and retains raw observations',async t=>{
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'wcv-rx-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  const testDir=path.join(root,'test');fs.mkdirSync(testDir);
  const listeners=new Set(),loadSteps=[];let seq=0,job=0,reg=0,pages=0,readySeen=false;
  const events={get seq(){return seq;},on(f){listeners.add(f);},off(f){listeners.delete(f);},emit(e){seq++;for(const f of listeners)f({...e,seq});}};
  const memory={0:'88 57',2:'88 00',24:'02 00 00 00',82:'34 12',84:'01 00',86:'02 00',88:'03 00',90:'04 00'};
  const invoke=async(method,params)=>{
    if(method==='instrument.list')return {data:{devices:[{kind:'usb_bridge',resource:'bridge://ft4222/serial=A',device_id:3,identity:{serial:'A'}},{kind:'oscilloscope',resource:'USB::SCOPE',device_id:5,identity:{serial:'S'}},{kind:'electronic_load',resource:'USB::LOAD',device_id:6}]}};
    if(method==='serial.ports')return {ok:true,data:[{device:'COM7'}]};
    if(method==='serial.monitor.status')return {data:{sessions:[]}};
    if(method==='serial.monitor.start')return {data:{port:'COM7',tab_id:0,already_open:false}};
    if(method==='serial.monitor.stop')return {data:{}};
    if(method==='log.lines.get'){
      pages++;
      if(pages===1)return {data:{lines:[],next_row:0,dropped:0}};
      readySeen=true;return {data:{lines:['Start CHG','ASK 71 22 1 5'],next_row:2,dropped:0}};
    }
    assert.equal(method,'instrument.command');
    if(Object.keys(params.command).some(k=>k.startsWith('Load'))) {
      assert.equal(readySeen,true,'load must wait for RX');assert.equal(params.device_id,6);loadSteps.push(params.command);job++;
      events.emit({type:'instrument.job_done',data:{job_id:job,ok:true}});return {data:{job_id:job}};
    }
    const c=params.command.BridgeI2c;assert(c);job++;
    assert.equal(readySeen,true,'I2C must wait for TX protocol evidence');
    if(c.write_hex)reg=Number.parseInt(c.write_hex,16);
    const wr=c.write_hex?c.write_hex.match(/../g).join(' ').toUpperCase():'—';
    const rd=c.read_len?memory[reg]:'—';
    events.emit({type:'instrument.job_done',data:{job_id:job,ok:true,detail:{response:`I2C 0x2B @100000Hz write ${wr} read ${rd}`}}});
    return {data:{job_id:job}};
  };
  const host=createFpgaHostAdapter({dataRoot:root,layout:{test_dir:testDir},invoke,events});
  host.capture=async({dir})=>{const f=path.join(dir,'shots','rx-observe.png');fs.mkdirSync(path.dirname(f),{recursive:true});fs.writeFileSync(f,'screenshot');return {screenshot:{path:f,sha256:'shot-hash'}};};
  host.waveformSource=async({dir})=>{const f=path.join(dir,'waves','rx-observe.isf');fs.mkdirSync(path.dirname(f),{recursive:true});fs.writeFileSync(f,'waveform');return {artifact:{path:f,sha256:'wave-hash'}};};
  host.measure=async({resource,unit})=>{await host.assertInstrumentLease(resource);assert.equal(unit,'V');return {value:6.5,unit:'V'};};
  const item=JSON.parse(fs.readFileSync(fromRepo('recipes/wireless-whitebox/cases/wcv_07.json')));
  item.limit={metric:'fop',unit:'raw',minimum:0x1200,maximum:0x1300};
  item.physical_vout_limit={minimum_v:5,maximum_v:6,settle_ms:0};
  const binding={values:[item],selected_item_ids:['wcv_07'],manifest_sha256:'hash',preflight_revision:'hash',run_id:'run'};
  const ctx={args:{mode:'rx_read'},dataRoot:root,invoke,fpgaAdapter:host,projectConfig:{value:{station:{station_id:'lab',rx_i2c_interface:'bridge://ft4222/serial=A',rx_i2c_clock_hz:100000,scope_resource:'USB::SCOPE',scope_capture_enabled:true,scope_source_enabled:true,load_resource:'USB::LOAD',log_source:'COM7',log_baud:115200}}},testMatrix:binding,plugin:{id:'wireless-whitebox'},matrixFile:path.join(testDir,'matrix.json'),layout:{test_dir:testDir,artifacts_dir:path.join(root,'artifacts')}};
  assert.equal((await preflight(ctx)).ok,true);assert.equal(job,0,'preflight cannot access I2C');
  const result=await run(ctx), observation=JSON.parse(fs.readFileSync(path.join(root,'artifacts/reports/rx-telemetry.json')));
  assert.equal(job,16);assert.equal(observation.telemetry.values_raw.fop,0x1234);assert.equal(observation.identity.variant,'HW');
  assert.equal(observation.captures.screenshot.path,'shots/rx-observe.png');assert.equal(observation.captures.waveform.path,'waves/rx-observe.isf');
  assert.equal(observation.check.verdict,'PASS');assert.equal(observation.check.scope,'internal_register_only');
  assert.equal(observation.physical_check.verdict,'FAIL');assert.equal(observation.physical_check.value,6.5);
  assert.deepEqual(observation.readiness.events.map(e=>e.marker),['Start CHG','ASK 71']);
  assert.equal(result.product_verdict,'FAIL');assert.equal(fs.existsSync(path.join(testDir,'wireless-rx-active.json')),false);
  assert.match(fs.readFileSync(result.summary_md,'utf8'),/仅采集 RX 内部寄存器/);
  assert.equal((await stop(ctx)).ok,true);
  // If screenshot export fails after a confirmed out-of-range reading, retain
  // that failure and the partial observation rather than replacing it by ERROR.
  const run2Dir=path.join(root,'projects/default/tests/WirelessWhitebox');fs.mkdirSync(run2Dir,{recursive:true});pages=0;readySeen=false;
  const host2=createFpgaHostAdapter({dataRoot:root,layout:{test_dir:run2Dir},invoke,events});
  host2.measure=async({resource})=>{await host2.assertInstrumentLease(resource);return {value:6.5,unit:'V'};};
  host2.capture=async()=>{const e=new Error('SCOPE.EXPORT_FAILED');e.quiescent=true;throw e;};
  const meta={revision:1,sha256:'b'.repeat(64),schema_hash:'c'.repeat(64)};
  const binding2={...binding,run_id:'run2',items:[{id:'wcv_07',sha256:'a'.repeat(64)}],project_config:meta};
  const ctx2={...ctx,project:'default',fpgaAdapter:host2,testMatrix:binding2,matrixFile:path.join(run2Dir,'matrix.json'),layout:{test_dir:run2Dir,artifacts_dir:path.join(run2Dir,'runs/run2/artifacts')}};
  await assert.rejects(run(ctx2),/SCOPE.EXPORT_FAILED/);
  const matrix2=JSON.parse(fs.readFileSync(ctx2.matrixFile));assert.equal(matrix2.items[0].status,'fail');
  const result2=JSON.parse(fs.readFileSync(path.join(run2Dir,'runs/run2/artifacts/reports/items/wcv_07/result.json')));
  assert.equal(result2.product_verdict,'FAIL');assert.equal(result2.execution_status,'error');assert.equal(result2.checks.find(c=>c.id==='rx_vout_instrument').value,6.5);
  assert(fs.existsSync(path.join(run2Dir,'runs/run2/artifacts/reports/items/wcv_07/partial-observation.json')));
  const loadDir=path.join(root,'projects/default/tests/Load');fs.mkdirSync(loadDir,{recursive:true});pages=0;readySeen=false;
  const loadHost=createFpgaHostAdapter({dataRoot:root,layout:{test_dir:loadDir},invoke,events});
  loadHost.measure=async({resource})=>{await loadHost.assertInstrumentLease(resource);loadSteps.push('measure');return {value:5.4,unit:'V'};};
  const loaded={...item,load_cc:{current_a:0.5,maximum_a:1}};
  const ctxLoad={...ctx,project:'default',args:{mode:'rx_load'},projectConfig:{value:{station:{...ctx.projectConfig.value.station,scope_capture_enabled:false,scope_source_enabled:false}}},fpgaAdapter:loadHost,testMatrix:{...binding2,values:[loaded],run_id:'load'},matrixFile:path.join(loadDir,'matrix.json'),layout:{test_dir:loadDir,artifacts_dir:path.join(loadDir,'runs/load/artifacts')}};
  assert.equal((await preflight(ctxLoad)).ok,true);const loadResult=await run(ctxLoad);
  assert.deepEqual(loadSteps,[{LoadInput:false},{LoadMode:'CC'},{LoadLevel:{mode:'CC',value:0.5}},{LoadInput:true},'measure',{LoadInput:false}]);
  const loadObservation=JSON.parse(fs.readFileSync(path.join(loadDir,'runs/load/artifacts/reports/rx-telemetry.json')));
  assert.equal(loadObservation.load.requested_current_a,0.5);assert.equal(loadObservation.load.physical_off_verified,false);
  assert.match(fs.readFileSync(loadResult.summary_md,'utf8'),/电子负载恒流设定 0.5 A，允许上限 1 A/);
  // Once a control write succeeds, failure to verify the physical output must
  // retain a station-wide recovery marker and never silently release the lease.
  const run3Dir=path.join(root,'projects/default/tests/Control');fs.mkdirSync(run3Dir,{recursive:true});pages=0;readySeen=false;
  const host3=createFpgaHostAdapter({dataRoot:root,layout:{test_dir:run3Dir},invoke,events});
  host3.measure=async()=>{const e=new Error('LOAD.READ_FAILED');e.quiescent=true;throw e;};
  const controlled={...item,rx_vout_control:{target_mv:5500,minimum_mv:5000,maximum_mv:6000}};
  const ctx3={...ctx,project:'default',args:{mode:'rx_control'},fpgaAdapter:host3,testMatrix:{...binding2,values:[controlled],run_id:'run3'},matrixFile:path.join(run3Dir,'matrix.json'),layout:{test_dir:run3Dir,artifacts_dir:path.join(run3Dir,'runs/run3/artifacts')}};
  await assert.rejects(run(ctx3),/LOAD.READ_FAILED/);
  const lock=JSON.parse(fs.readFileSync(path.join(run3Dir,'.leases/station-lab.json')));
  assert.equal(lock.recovery_reason,'RX_OUTPUT_UNVERIFIED');
  assert(fs.existsSync(path.join(root,'.station-recovery.json')));
});

test('RX preflight rejects missing device or mixed test selection without I2C',async()=>{
  const ctx={args:{mode:'rx_read'},projectConfig:{value:{station:{station_id:'lab',rx_i2c_interface:'bridge://missing'}}},testMatrix:{selected_item_ids:['wcv_07'],manifest_sha256:'x'},invoke:async()=>({data:{devices:[]}})};
  assert.equal((await preflight(ctx)).ok,false);
  ctx.testMatrix.selected_item_ids.push('wcv_20');assert.match((await preflight(ctx)).error,/仅选择/);
});

test('RX output control rejects missing or inverted limits before any device call',async()=>{
  let calls=0;const item={id:'wcv_07',rx_vout_control:{target_mv:9000,minimum_mv:5000,maximum_mv:6000}};
  const ctx={args:{mode:'rx_control'},testMatrix:{values:[item],selected_item_ids:['wcv_07'],manifest_sha256:'x'},invoke:async()=>{calls++;throw new Error('must not access hardware');}};
  assert.match((await preflight(ctx)).error,/RX.VOUT_LIMIT_REQUIRED/);
  await assert.rejects(run(ctx),/RX.VOUT_LIMIT_REQUIRED/);assert.equal(calls,0);
  item.rx_vout_control.target_mv=5500;
  assert.match((await preflight(ctx)).error,/LOAD.LIMIT_REQUIRED/);
  await assert.rejects(run(ctx),/LOAD.LIMIT_REQUIRED/);assert.equal(calls,0);
});

for(const mode of ['stale','dropped']) test(`RX ${mode} log never triggers I2C`,async t=>{
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'wcv-rx-gate-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  const testDir=path.join(root,'test');fs.mkdirSync(testDir);let page=0,commands=0;
  const invoke=async(method,params)=>{
    if(method==='instrument.list')return {data:{devices:[{kind:'usb_bridge',resource:'bridge://ft4222/serial=A',device_id:3}]}};
    if(method==='serial.ports')return {data:{ports:[{device:'COM7'}]}};
    if(method==='serial.monitor.status')return {data:{sessions:[]}};
    if(method==='serial.monitor.start')return {data:{port:'COM7',tab_id:0}};
    if(method==='serial.monitor.stop')return {data:{}};
    if(method==='log.lines.get'){
      page++;
      if(page===1)return {data:{lines:['Start CHG','ASK 71'],next_row:2,dropped:0}};
      assert.equal(params.from_row,2);
      return {data:{lines:[],next_row:2,dropped:mode==='dropped'?1:0}};
    }
    if(method==='instrument.command'){commands++;throw new Error('unexpected I2C');}
    throw new Error(method);
  };
  const host=createFpgaHostAdapter({dataRoot:root,layout:{test_dir:testDir},invoke});
  const item=JSON.parse(fs.readFileSync(fromRepo('recipes/wireless-whitebox/cases/wcv_07.json')));
  const ctx={args:{mode:'rx_read'},dataRoot:root,invoke,fpgaAdapter:host,projectConfig:{value:{station:{station_id:'lab',rx_i2c_interface:'bridge://ft4222/serial=A',log_source:'COM7',rx_ready_timeout_s:1}}},testMatrix:{values:[item],selected_item_ids:['wcv_07'],manifest_sha256:'x',run_id:'run'},plugin:{id:'wireless-whitebox'},matrixFile:path.join(testDir,'matrix.json'),layout:{test_dir:testDir,artifacts_dir:path.join(root,'artifacts')}};
  await assert.rejects(run(ctx),mode==='dropped'?/LOG.DROPPED/:/RX.READY_TIMEOUT/);
  assert.equal(commands,0);
});

test('stop while waiting for RX identification prevents I2C and releases serial',async t=>{
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'wcv-rx-stop-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  const testDir=path.join(root,'test');fs.mkdirSync(testDir);let page=0,commands=0,serialStopped=false,wake;
  const observedStart=new Promise(resolve=>{wake=resolve;});
  const invoke=async(method)=>{
    if(method==='instrument.list')return {data:{devices:[{kind:'usb_bridge',resource:'bridge://ft4222/serial=A',device_id:3}]}};
    if(method==='serial.ports')return {data:{ports:[{device:'COM7'}]}};
    if(method==='serial.monitor.status')return {data:{sessions:[]}};
    if(method==='serial.monitor.start')return {data:{port:'COM7',tab_id:0}};
    if(method==='serial.monitor.stop'){serialStopped=true;return {data:{}};}
    if(method==='log.lines.get'){page++;if(page===1)return {data:{lines:[],next_row:0,dropped:0}};wake();return {data:{lines:['Start CHG'],next_row:1,dropped:0}};}
    if(method==='instrument.command'){commands++;throw new Error('unexpected I2C');}
    throw new Error(method);
  };
  const host=createFpgaHostAdapter({dataRoot:root,layout:{test_dir:testDir},invoke});
  const item=JSON.parse(fs.readFileSync(fromRepo('recipes/wireless-whitebox/cases/wcv_07.json')));
  const ctx={args:{mode:'rx_read'},dataRoot:root,invoke,fpgaAdapter:host,projectConfig:{value:{station:{station_id:'lab',rx_i2c_interface:'bridge://ft4222/serial=A',log_source:'COM7',rx_ready_timeout_s:5}}},testMatrix:{values:[item],selected_item_ids:['wcv_07'],manifest_sha256:'x',run_id:'run'},plugin:{id:'wireless-whitebox'},matrixFile:path.join(testDir,'matrix.json'),layout:{test_dir:testDir,artifacts_dir:path.join(root,'artifacts')}};
  const running=run(ctx);await observedStart;await stop(ctx);await assert.rejects(running,/RUN.ABORTED/);
  assert.equal(commands,0);assert.equal(serialStopped,true);assert.equal(fs.existsSync(path.join(testDir,'wireless-rx-active.json')),false);
});

test('real runner completes RX read against WiParse HTTP and event stream simulator',async t=>{
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'wcv-rx-runner-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  fs.cpSync(fromRepo('recipes/wireless-whitebox'),path.join(root,'recipes/wireless-whitebox'),{recursive:true});
  const params={mode:'rx_read'}, context={pluginDir:fromRepo('test-tools/plugins/wireless-whitebox'),dataRoot:root,project:'default',testId:'WirelessWhitebox',params};
  const project=execute({context,method:'project.get',internal:true});
  const value=structuredClone(project.value);Object.assign(value.station,{station_id:'lab',rx_i2c_interface:'bridge://ft4222/serial=A',log_source:'COM7',load_resource:'USB::LOAD'});
  execute({context,method:'project.save',params:{value,expected_revision:0,expected_sha256:''},internal:true});
  const initialCase=execute({context,method:'config.get',params:{config_id:'wcv_07'},internal:true});
  execute({context,method:'config.save',params:{config_id:'wcv_07',value:{...initialCase.value,physical_vout_limit:{minimum_v:5,maximum_v:6,settle_ms:0}},expected_revision:initialCase.revision,expected_sha256:initialCase.sha256},internal:true});
  const sockets=new Set(),history=[],commands=[];let seq=0,job=0,reg=0,pages=0,simulateDrop=false,failControl=false;
  const memory={0:'88 57',2:'88 00',24:'02 00 00 00',82:'34 12',84:'01 00',86:'02 00',88:'03 00',90:'04 00'};
  const server=http.createServer(async(req,res)=>{
    if(req.url.startsWith('/v1/events')) {
      res.writeHead(200,{'Content-Type':'application/x-ndjson'});res.flushHeaders();sockets.add(res);res.on('close',()=>sockets.delete(res));
      const since=Number(new URL(req.url,'http://local').searchParams.get('since_seq')||0);for(const e of history.filter(e=>e.seq>since))res.write(JSON.stringify(e)+'\n');return;
    }
    if(req.url==='/v1/health'){res.setHeader('Content-Type','application/json');res.end(JSON.stringify({ok:true,version:'1.2.2'}));return;}
    let raw='';for await(const part of req)raw+=part;const {method,params:p}=JSON.parse(raw);let data={};
    if(method==='instrument.list')data={devices:[{kind:'usb_bridge',resource:'bridge://ft4222/serial=A',device_id:3,identity:{serial:'A'}},{kind:'electronic_load',resource:'USB::LOAD',device_id:5,identity:{serial:'L'}}]};
    else if(method==='serial.ports')data={ports:[{device:'COM7'}]};
    else if(method==='serial.monitor.status')data={sessions:[]};
    else if(method==='serial.monitor.start')data={port:'COM7',tab_id:0};
    else if(method==='serial.monitor.stop')data={};
    else if(method==='log.lines.get'){pages++;data=pages===1?{lines:[],next_row:0,dropped:0}:simulateDrop?{lines:[],next_row:0,dropped:1}:{lines:['Start CHG','ASK 71 22'],next_row:2,dropped:0};}
    else if(method==='instrument.measure'){
      assert.equal(p.device_id,5);const event={seq:++seq,type:'instrument.measurements',data:{device_id:5,readings:[{channel:'Current',value:0.5,unit:'A'},{channel:'Voltage',value:5.4,unit:'V'}]}};
      history.push(event);setTimeout(()=>{for(const stream of sockets)stream.write(JSON.stringify(event)+'\n');},10);data={accepted:true};
    }
    else if(method==='instrument.command'){
      const c=p.command.BridgeI2c;if(!c)throw new Error('unexpected command');job++;
      commands.push(c);
      if(c.write_hex)reg=Number.parseInt(c.write_hex,16);
      const wr=c.write_hex?c.write_hex.match(/../g).join(' ').toUpperCase():'—',rd=c.read_len?memory[reg]:'—';
      const failed=failControl&&c.write_hex==='00be7c15';
      const event={seq:++seq,type:'instrument.job_done',data:{job_id:job,ok:!failed,detail:failed?{message:'simulated write error'}:{response:`I2C 0x2B @100000Hz write ${wr} read ${rd}`}}};
      history.push(event);setTimeout(()=>{for(const stream of sockets)stream.write(JSON.stringify(event)+'\n');},10);data={accepted:true,job_id:job};
    } else throw new Error('unexpected method '+method);
    res.setHeader('Content-Type','application/json');res.end(JSON.stringify({ok:true,data}));
  });
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));t.after(()=>{for(const socket of sockets)socket.destroy();server.close();});
  const overlay=path.join(root,'overlay.json');fs.writeFileSync(overlay,JSON.stringify({params,test_matrix:{selected_item_ids:['wcv_07']}}));
  const base=[fromRepo('test-tools/runner.mjs'),'--plugin','wireless-whitebox','--data-root',root,'--url',`http://127.0.0.1:${server.address().port}`,'--overlay',overlay];
  const launch=args=>new Promise((resolve,reject)=>{const child=spawn(process.execPath,[...base,...args],{windowsHide:true});let output='';const timer=setTimeout(()=>child.kill(),15000);child.stdout.on('data',x=>output+=x);child.stderr.on('data',x=>output+=x);child.on('error',reject);child.on('close',code=>{clearTimeout(timer);resolve({code,output});});});
  const pre=await launch(['--lifecycle','preflight']);assert.equal(pre.code,0,pre.output);assert.equal(job,0);
  const state=execute({context,method:'preflight.state',internal:true});assert.equal(state.valid,true);
  fs.writeFileSync(overlay,JSON.stringify({params,test_matrix:{selected_item_ids:['wcv_07'],config_manifest_sha256:state.manifest_sha256,preflight_revision:state.revision}}));
  const result=await launch(['--lifecycle','run','--stamp','rx_test']);assert.equal(result.code,0,result.output);assert.equal(job,16);
  const matrix=execute({context,method:'matrix.get',internal:true}),row=matrix.items.find(i=>i.id==='wcv_07');
  const detail=execute({context,method:'result.get',params:{config_id:row.id,run_id:matrix.run_id,attempt:row.attempt},internal:true});
  assert.equal(detail.available,true);assert(detail.evidence.some(e=>e.label==='rx_telemetry'&&e.available));assert.equal(detail.value.product_verdict,'INCONCLUSIVE');
  assert.equal(detail.value.checks.find(c=>c.id==='rx_vout_instrument').value,5.4);
  pages=0;simulateDrop=true;
  const failed=await launch(['--lifecycle','run','--stamp','rx_fail']);assert.notEqual(failed.code,0);assert.equal(job,16,'dropped log must not start another I2C job');
  const matrix2=execute({context,method:'matrix.get',params:{refresh:true},internal:true}),row2=matrix2.items.find(i=>i.id==='wcv_07');
  assert.equal(row2.status,'error');
  const errorDetail=execute({context,method:'result.get',params:{config_id:row2.id,run_id:matrix2.run_id,attempt:row2.attempt},internal:true});
  assert.equal(errorDetail.available,true);assert.equal(errorDetail.value.product_verdict,'INCONCLUSIVE');assert.equal(errorDetail.value.message,'LOG.DROPPED');
  const current=execute({context,method:'config.get',params:{config_id:'wcv_07'},internal:true});
  execute({context,method:'config.save',params:{config_id:'wcv_07',value:{...current.value,limit:{metric:'fop',unit:'raw',minimum:0,maximum:1000},rx_vout_control:{target_mv:5500,minimum_mv:5000,maximum_mv:6000}},expected_revision:current.revision,expected_sha256:current.sha256},internal:true});
  params.mode='rx_control';pages=0;simulateDrop=false;
  fs.writeFileSync(overlay,JSON.stringify({params,test_matrix:{selected_item_ids:['wcv_07']}}));
  const preControl=await launch(['--lifecycle','preflight']);assert.equal(preControl.code,0,preControl.output);
  const controlState=execute({context,method:'preflight.state',internal:true});
  fs.writeFileSync(overlay,JSON.stringify({params,test_matrix:{selected_item_ids:['wcv_07'],config_manifest_sha256:controlState.manifest_sha256,preflight_revision:controlState.revision}}));
  const controlRun=await launch(['--lifecycle','run','--stamp','rx_control']);assert.equal(controlRun.code,0,controlRun.output);
  assert.equal(job,48);assert(commands.some(c=>c.write_hex==='00be7c15'));assert(commands.some(c=>c.write_hex==='002c00080000'));
  const rx=JSON.parse(fs.readFileSync(path.join(root,'projects/default/tests/WirelessWhitebox/runs/rx_control/artifacts/reports/rx-telemetry.json')));
  assert.equal(rx.control.requested_mv,5500);assert.equal(rx.control.physical_output_verified,false);
  assert.equal(rx.check.verdict,'FAIL');
  const failureMatrix=execute({context,method:'matrix.get',params:{refresh:true},internal:true});assert.equal(failureMatrix.items.find(i=>i.id==='wcv_07').status,'fail');
  pages=0;failControl=true;
  const failedControl=await launch(['--lifecycle','run','--stamp','rx_control_fail']);assert.notEqual(failedControl.code,0);
  const lock=JSON.parse(fs.readFileSync(path.join(root,'projects/default/tests/WirelessWhitebox/.leases/station-lab.json')));
  assert.equal(lock.manual_recovery_required,true);assert.equal(lock.recovery_reason,'RX_OUTPUT_STATE_UNKNOWN');
  const commandCount=job;pages=0;
  const retry=await launch(['--lifecycle','run','--stamp','rx_control_retry']);assert.notEqual(retry.code,0);assert.equal(job,commandCount,'uncertain output must not be retried');
});
