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
import run, {configure,preflight} from '../plugins/wireless-whitebox/index.mjs';

test('limit configuration rejects an inverted range',async()=>{
  const result=await configure({overlay:{config:{limit:{minimum:10,maximum:1}}}});
  assert.equal(result.ok,false);assert.equal(result.errors[0].path,'/limit');
});

test('wireless RX limits save independently, preserve case data and reject stale or unsafe changes',t=>{
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'wcv-limits-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  const cases=path.join(root,'cases');fs.cpSync(fromRepo('recipes/wireless-whitebox/cases'),cases,{recursive:true});
  const context={pluginDir:fromRepo('test-tools/plugins/wireless-whitebox'),dataRoot:root,project:'default',testId:'WirelessWhitebox',params:{config_dir:cases},activePlugin:'wireless-whitebox'};
  const call=(method,params)=>execute({context,method,params,internal:false});
  const row=call('ui.test_tool.config.get',{config_id:'wcv_07'}),before=JSON.parse(fs.readFileSync(path.join(cases,'wcv_07.json')));
  const base={config_id:'wcv_07',expected_project:'default',expected_revision:row.revision,expected_sha256:row.sha256};
  const saved=call('ui.test_tool.config.patch',{...base,patch:{wireless_limits:{rx_vout_control:{target_mv:5000,minimum_mv:4500,maximum_mv:5500}}}});
  assert.equal(saved.value.rx_vout_control.target_mv,5000);assert.equal(saved.value.objective,before.objective);
  assert.throws(()=>call('ui.test_tool.config.patch',{...base,patch:{wireless_limits:{load_cc:{current_a:1,maximum_a:2}}}}),e=>e.code==='CONFIG.REVISION_CONFLICT');
  const next={...base,expected_revision:saved.revision,expected_sha256:saved.sha256};
  assert.throws(()=>call('ui.test_tool.config.patch',{...next,patch:{wireless_limits:{load_cc:{current_a:2,maximum_a:1}}}}),e=>e.code==='CONFIG.INVALID_REQUEST');
  const after=call('ui.test_tool.config.patch',{...next,patch:{wireless_limits:{physical_vout_limit:{minimum_v:4.5,maximum_v:5.5,settle_ms:300}}}});
  assert.equal(after.value.rx_vout_control.target_mv,5000);assert.equal(after.value.physical_vout_limit.maximum_v,5.5);
  assert.throws(()=>call('ui.test_tool.config.patch',{...next,config_id:'wcv_01',patch:{wireless_limits:{load_cc:{current_a:1,maximum_a:2}}}}),e=>e.code==='CONFIG.INVALID_REQUEST');
});

test('wireless station selection preserves other settings, saves seed and checks revisions',t=>{
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'wcv-station-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  fs.cpSync(fromRepo('recipes/wireless-whitebox'),path.join(root,'recipes/wireless-whitebox'),{recursive:true});
  const context={pluginDir:fromRepo('test-tools/plugins/wireless-whitebox'),dataRoot:root,project:'default',testId:'WirelessWhitebox',params:{},activePlugin:'wireless-whitebox'};
  const call=(method,params={})=>execute({context,method,params,internal:false}),before=call('ui.test_tool.project.get');
  assert.equal(before.saved,false);assert.equal(before.revision,0);
  const base={expected_project:'default',expected_revision:0,expected_sha256:''};
  const saved=call('ui.test_tool.project.patch_wireless_station',{...base,patch:{station_id:'LAB-01',rx_i2c_interface:'bridge://ft4222/serial=FT1',log_source:'COM3',log_baud:115200}});
  assert.equal(saved.revision,1);assert.equal(saved.value.station.rx_i2c_interface,'bridge://ft4222/serial=FT1');assert.deepEqual(saved.value.product,before.value.product);assert.equal(saved.value.station.rx_running_marker,before.value.station.rx_running_marker);
  assert.throws(()=>call('ui.test_tool.project.patch_wireless_station',{...base,patch:{station_id:'stale'}}),e=>e.code==='CONFIG.REVISION_CONFLICT');
  const next={expected_project:'default',expected_revision:saved.revision,expected_sha256:saved.sha256};
  assert.throws(()=>call('ui.test_tool.project.patch_wireless_station',{...next,patch:{scope_source_enabled:true,scope_capture_enabled:false}}),e=>e.code==='CONFIG.INVALID_REQUEST');
  assert.throws(()=>call('ui.test_tool.project.patch_wireless_station',{...next,patch:{log_source:'[LOCAL_PATH]'}}),e=>e.code==='CONFIG.INVALID_REQUEST');
  const current=call('ui.test_tool.project.get');assert.equal(current.value.station.station_id,'LAB-01');assert.equal(current.revision,1);
});

test('20 templates load through the Hub and review cannot certify a DUT', async t => {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'wireless-whitebox-'));
  t.after(() => fs.rmSync(root, {recursive:true, force:true}));
  const cases = path.join(root, 'cases');
  fs.cpSync(fromRepo('recipes/wireless-whitebox/cases'), cases, {recursive:true});
  const context = {pluginDir:fromRepo('test-tools/plugins/wireless-whitebox'), dataRoot:root, project:'default', testId:'WirelessWhitebox', params:{config_dir:cases}};
  const list = execute({context, method:'config.list', params:{}, internal:true});
  assert.equal(list.total,20);
  assert(list.items.every(item=>item.status==='valid'));
  const values = list.items.map(item=>JSON.parse(fs.readFileSync(path.join(cases, item.id+'.json'))));
  const binding = {values, selected_item_ids:values.map(v=>v.id), manifest_sha256:'manifest', preflight_revision:'manifest', run_id:'test-run'};
  assert.equal((await preflight({args:{mode:'live'},testMatrix:binding,invoke:async()=>({data:{devices:[],ports:[]}})})).ok,false);
  assert.equal((await preflight({args:{mode:'review'},testMatrix:binding})).ok,true);
  const ctx={args:{mode:'review'},testMatrix:binding,plugin:{id:'wireless-whitebox'},matrixFile:path.join(root,'matrix.json'),layout:{artifacts_dir:path.join(root,'artifacts')}};
  const result=await run(ctx);
  assert.equal(result.product_verdict,'INCONCLUSIVE');
  const matrix=JSON.parse(fs.readFileSync(ctx.matrixFile));
  assert.equal(matrix.summary.inconclusive,20);
  assert.equal(matrix.summary.pass,0);
  assert.match(fs.readFileSync(result.summary_md,'utf8'),/未连接设备/);
  await assert.rejects(run({...ctx,args:{mode:'live'}}),/LIVE_BACKEND_NOT_VALIDATED/);
});

test('runner binds a saved project and freezes all 20 configurations', async t => {
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'wcv-runner-'));
  t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  fs.cpSync(fromRepo('recipes/wireless-whitebox'),path.join(root,'recipes/wireless-whitebox'),{recursive:true});
  const context={pluginDir:fromRepo('test-tools/plugins/wireless-whitebox'),dataRoot:root,project:'default',testId:'WirelessWhitebox',params:{}};
  const project=execute({context,method:'project.get',internal:true});
  assert.equal(project.saved,false);
  execute({context,method:'project.save',params:{value:project.value,expected_revision:0,expected_sha256:''},internal:true});
  const server=http.createServer((req,res)=>{res.setHeader('Content-Type','application/json');res.end(JSON.stringify({ok:true,version:'1.2.2',data:{version:'1.2.2'}}));});
  await new Promise(resolve=>server.listen(0,'127.0.0.1',resolve));t.after(()=>server.close());
  const base=[fromRepo('test-tools/runner.mjs'),'--plugin','wireless-whitebox','--data-root',root,'--url',`http://127.0.0.1:${server.address().port}`];
  const launch=args=>new Promise((resolve,reject)=>{const child=spawn(process.execPath,[...base,...args],{windowsHide:true});let output='';child.stdout.on('data',x=>output+=x);child.stderr.on('data',x=>output+=x);child.on('error',reject);child.on('close',code=>resolve({code,output}));});
  const pre=await launch(['--lifecycle','preflight']);assert.equal(pre.code,0,pre.output);
  const state=execute({context,method:'preflight.state',internal:true});assert.equal(state.valid,true);
  const overlay=path.join(root,'overlay.json');fs.writeFileSync(overlay,JSON.stringify({test_matrix:{config_manifest_sha256:state.manifest_sha256,preflight_revision:state.revision,selected_item_ids:state.selected_item_ids}}));
  const result=await launch(['--lifecycle','run','--stamp','review','--overlay',overlay]);assert.equal(result.code,0,result.output);
  const matrix=execute({context,method:'matrix.get',internal:true});assert.equal(matrix.available,true,JSON.stringify(matrix));assert.equal(matrix.summary.inconclusive,20);assert.equal(matrix.summary.pass,0);
  const runDir=path.join(root,'projects/default/tests/WirelessWhitebox/runs/review');
  assert.equal(fs.readdirSync(path.join(runDir,'config-snapshot')).length,20);
  assert.equal(JSON.parse(fs.readFileSync(path.join(runDir,'project-config.json'))).revision,1);
  assert.equal(JSON.parse(fs.readFileSync(path.join(runDir,'run.json'))).product_verdict,'INCONCLUSIVE');
  const row=execute({context,method:'config.get',params:{config_id:'wcv_01'},internal:true});
  execute({context,method:'config.save',params:{config_id:'wcv_01',value:{...row.value,name:'更新启动'},expected_revision:row.revision,expected_sha256:row.sha256},internal:true});
  const stale=await launch(['--lifecycle','run','--stamp','stale','--overlay',overlay]);assert.notEqual(stale.code,0);assert.match(stale.output,/PREFLIGHT/);
});

test('log analysis freezes source bytes, reports observed recovery and rejects changed inputs', async t=>{
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'wcv-log-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  const testDir=path.join(root,'test');fs.mkdirSync(testDir);
  const input=path.join(root,'input.txt');
  const raw='[17:27:47.726]收←◆FodCnt = 5\nFO Alert!\n[17:27:48.026]收←◆EPT_REASON:2000\n[17:27:58.100]收←◆Start CHG\n';
  fs.writeFileSync(input,raw);
  const item={id:'wcv_20',name:'历史故障复现',tags:[],stimulus:'观察',internal_evidence:'串口',physical_evidence:'待测',judgment:'待确认'};
  const binding={values:[item],selected_item_ids:['wcv_20'],manifest_sha256:'hash',preflight_revision:'hash',run_id:'run'};
  const ctx={args:{mode:'log_analysis',log_file:input},dataRoot:root,testMatrix:binding,plugin:{id:'wireless-whitebox'},matrixFile:path.join(testDir,'matrix.json'),layout:{test_dir:testDir,artifacts_dir:path.join(root,'artifacts')}};
  assert.equal((await preflight(ctx)).ok,true);
  const result=await run(ctx);const report=fs.readFileSync(result.summary_md,'utf8');
  assert.match(report,/FodCnt=5/);assert.match(report,/EPT_REASON=2000/);assert.match(report,/17:27:58.100/);
  assert.equal(fs.readFileSync(path.join(root,'artifacts/reports/source-log.txt'),'utf8'),raw);
  assert.equal(result.product_verdict,'INCONCLUSIVE');
  fs.appendFileSync(input,'changed');await assert.rejects(run(ctx),/LOG.PREFLIGHT_STALE/);
  const escaped=await preflight({...ctx,args:{mode:'log_analysis',log_file:path.join(os.tmpdir(),'outside.txt')}});assert.equal(escaped.ok,false);
});

test('real runner executes selected FOD log analysis with frozen project and evidence',async t=>{
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'wcv-runner-log-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  fs.cpSync(fromRepo('recipes/wireless-whitebox'),path.join(root,'recipes/wireless-whitebox'),{recursive:true});
  if(process.env.WIPARSE_ANALYSIS_SAMPLE)fs.copyFileSync(process.env.WIPARSE_ANALYSIS_SAMPLE,path.join(root,'input.txt'));
  else fs.writeFileSync(path.join(root,'input.txt'),'[17:27:47.726]收←◆FodCnt = 5\nFO Alert!\n[17:27:48.026]收←◆EPT_REASON:2000\n[17:27:58.100]收←◆Start CHG\n');
  const params={mode:'log_analysis',log_file:path.join(root,'input.txt')};
  const context={pluginDir:fromRepo('test-tools/plugins/wireless-whitebox'),dataRoot:root,project:'default',testId:'WirelessWhitebox',params};
  const p=execute({context,method:'project.get',internal:true});execute({context,method:'project.save',params:{value:p.value,expected_revision:0,expected_sha256:''},internal:true});
  const server=http.createServer((req,res)=>{res.setHeader('Content-Type','application/json');res.end(JSON.stringify({ok:true,version:'1.2.2',data:{version:'1.2.2'}}));});await new Promise(r=>server.listen(0,'127.0.0.1',r));t.after(()=>server.close());
  const overlay=path.join(root,'overlay.json');fs.writeFileSync(overlay,JSON.stringify({params,test_matrix:{selected_item_ids:['wcv_20']}}));
  const base=[fromRepo('test-tools/runner.mjs'),'--plugin','wireless-whitebox','--data-root',root,'--url',`http://127.0.0.1:${server.address().port}`,'--overlay',overlay];
  const launch=args=>new Promise((resolve,reject)=>{const child=spawn(process.execPath,[...base,...args],{windowsHide:true});let output='';child.stdout.on('data',x=>output+=x);child.stderr.on('data',x=>output+=x);child.on('error',reject);child.on('close',code=>resolve({code,output}));});
  const pre=await launch(['--lifecycle','preflight']);assert.equal(pre.code,0,pre.output);
  const state=execute({context,method:'preflight.state',internal:true});assert.equal(state.valid,true);
  fs.writeFileSync(overlay,JSON.stringify({params,test_matrix:{selected_item_ids:['wcv_20'],config_manifest_sha256:state.manifest_sha256,preflight_revision:state.revision}}));
  const result=await launch(['--lifecycle','run','--stamp','log_test']);assert.equal(result.code,0,result.output);
  const analysis=JSON.parse(fs.readFileSync(path.join(root,'projects/default/tests/WirelessWhitebox/runs/log_test/artifacts/reports/log-analysis.json')));
  assert(analysis.candidates.length>=1);assert.equal(analysis.product_verdict,'INCONCLUSIVE');
  const matrix=execute({context,method:'matrix.get',internal:true});assert.equal(matrix.summary.inconclusive,1);assert.equal(matrix.summary.skipped,19);
  const row=matrix.items.find(item=>item.id==='wcv_20');
  const detail=execute({context,method:'result.get',params:{config_id:'wcv_20',run_id:matrix.run_id,attempt:row.attempt},internal:true});
  assert.equal(detail.available,true);assert.equal(detail.value.candidate_count,analysis.candidates.length);assert.equal(detail.value.product_verdict,'INCONCLUSIVE');
  assert(detail.evidence.some(e=>e.label==='analysis'&&e.available));
});
