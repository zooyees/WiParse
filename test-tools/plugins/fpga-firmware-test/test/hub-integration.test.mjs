import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {spawn} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {loadSchema} from '../../../lib/hub/schema.mjs';
import {validateMatrix} from '../../../lib/hub/matrix.mjs';
import {execute} from '../../../hub-workspace.mjs';
const pluginDir=fileURLToPath(new URL('..',import.meta.url)),repo=fileURLToPath(new URL('../../../../',import.meta.url));
test('delivered plugin runs through real Hub/runner with five simulated cases and navigable evidence',async t=>{
 const root=fs.mkdtempSync(path.join(os.tmpdir(),'fpga-hub-acceptance-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
 const output=await new Promise((resolve,reject)=>{const child=spawn(process.execPath,[path.join(repo,'scripts/generate-fpga-hub-demo.mjs'),root],{windowsHide:true});let output='';child.stdout.on('data',b=>output+=b);child.stderr.on('data',b=>output+=b);child.on('error',reject);child.on('close',code=>code===0?resolve(output):reject(Error(output)));});
 assert.equal(JSON.parse(output.trim()).count,5);
 const fixture=JSON.parse(fs.readFileSync(path.join(root,'fpga-ui-workspace.json')));
 const matrix=fixture.workspace.matrix;assert.equal(matrix.items.length,5);assert.equal(matrix.summary.pass,5);assert.equal(matrix.error,null);
 const testDir=path.join(root,'projects/default/tests/FpgaFirmwareTest'),snapshot=JSON.parse(fs.readFileSync(path.join(testDir,'matrix.json')));
 validateMatrix(snapshot,{pluginId:'fpga-firmware-test',projectRoot:path.join(root,'projects/default')});
 const schema=loadSchema(path.join(pluginDir,'schemas/fpga-matrix-status.schema.json'),pluginDir);assert.deepEqual(schema.errors(snapshot),[]);
 const runDir=path.join(testDir,'runs/20261018_150000_simulation'),meta=JSON.parse(fs.readFileSync(path.join(runDir,'run.json')));assert.equal(meta.simulation_only,true);assert.equal(meta.industrial_approved,false);assert.equal(meta.plugin_version,'0.5.0');
 const report=path.join(runDir,'artifacts/reports/summary.md'),md=fs.readFileSync(report,'utf8');assert.match(md,/模拟演示/);
 const links=[...md.matchAll(/\]\(([^)]+)\)/g)].map(m=>decodeURI(m[1]));assert.equal(links.length,20);for(const link of links)assert(fs.existsSync(path.resolve(path.dirname(report),link)),link);
 const context={pluginDir,dataRoot:root,project:'default',testId:'FpgaFirmwareTest',params:fixture.params};
 const listed=execute({context,method:'config.list',params:{limit:500}});assert.equal(listed.items.length,5);
});


test('simulated FAIL remains a product failure with two selected and three skipped rows',async t=>{
 const root=fs.mkdtempSync(path.join(os.tmpdir(),'fpga-hub-failure-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
 await new Promise((resolve,reject)=>{const child=spawn(process.execPath,[path.join(repo,'scripts/generate-fpga-hub-demo.mjs'),root,'fail','2'],{windowsHide:true});let output='';child.stdout.on('data',b=>output+=b);child.stderr.on('data',b=>output+=b);child.on('error',reject);child.on('close',code=>code===0?resolve():reject(Error(output)));});
 const fixture=JSON.parse(fs.readFileSync(path.join(root,'fpga-ui-workspace.json'))),summary=fixture.workspace.matrix.summary;
 assert.equal(summary.total,5);assert.equal(summary.selected,2);assert.equal(summary.fail,2);assert.equal(summary.skipped,3);assert.equal(summary.pass,0);
 const meta=JSON.parse(fs.readFileSync(path.join(root,'projects/default/tests/FpgaFirmwareTest/runs/20261018_150000_simulation/run.json')));assert.equal(meta.execution_status,'completed');assert.equal(meta.product_verdict,'FAIL');assert.equal(meta.simulation_only,true);
});

test('generated field labels remain independent across reused primitive and role schemas',()=>{
 const schema=JSON.parse(fs.readFileSync(path.join(pluginDir,'schemas/fpga-test-case.schema.json'),'utf8'));
 const shared=JSON.parse(fs.readFileSync(path.join(pluginDir,'schemas/project.schema.json'),'utf8'));
 assert.equal(shared.properties.dut.properties.model.title,'model');
 assert.equal(shared.properties.dut.properties.fpga_model.title,'fpga model');
 assert.equal(shared.properties.instruments.properties.scope.title,'scope');
 assert.equal(shared.properties.instruments.properties.log.title,'log');
 assert.equal(schema.$defs.rule.oneOf.find(s=>s.properties.type.const==='file_exists').properties.evidence_ref.title,'evidence ref');
});

test('status replacement retries transient Windows sharing and preserves old data on persistent failure',async t=>{
 const {atomic}=await import('../lib/common.mjs');
 const root=fs.mkdtempSync(path.join(os.tmpdir(),'fpga-atomic-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
 const file=path.join(root,'status.json');atomic(file,{revision:1});let attempts=0;
 atomic(file,{revision:2},(from,to)=>{if(++attempts<3)throw Object.assign(Error('sharing'),{code:'EPERM'});fs.renameSync(from,to)},()=>{});
 assert.equal(attempts,3);assert.equal(JSON.parse(fs.readFileSync(file)).revision,2);
 assert.throws(()=>atomic(file,{revision:3},()=>{throw Object.assign(Error('locked'),{code:'EPERM'})},()=>{}),{code:'EPERM'});
 assert.equal(JSON.parse(fs.readFileSync(file)).revision,2);assert.deepEqual(fs.readdirSync(root),['status.json']);
});
