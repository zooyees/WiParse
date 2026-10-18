import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import run,{preflight} from '../index.mjs';
import {projectRecord} from './fixtures/project-fixture.mjs';
import {planProjectMigration} from '../lib/project-migration.mjs';
import {digest} from '../lib/common.mjs';
function fixture(t){
 const root=fs.mkdtempSync(path.join(process.cwd(),'fpga-project-run-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
 const legacy=JSON.parse(fs.readFileSync(new URL('fixtures/legacy-demo-cases.json',import.meta.url)))[0];
 legacy.timeout_s=5;legacy.log_capture.duration_s=0.01;legacy.waveform_captures.forEach(w=>{w.at_s=0;w.timeout_s=0.5;});
 const plan=planProjectMigration({cases:[legacy],projectConfig:null}),cases=path.join(root,'cases'),testDir=path.join(root,'test');fs.mkdirSync(cases);
 fs.mkdirSync(path.join(root,'firmware'));fs.copyFileSync(new URL('../../../../recipes/fpga-firmware-test/firmware/demo-ip-suite.bin',import.meta.url),path.join(root,'firmware/demo-ip-suite.bin'));
 const item=plan.items[0].value,file=path.join(cases,item.id+'.json');fs.writeFileSync(file,JSON.stringify(item));
 const shared=projectRecord(plan.project_value,{projectId:'a',testId:'test'});
 const ctx={project:'a',projectConfig:shared,repositoryRoot:root,testDir,layout:{run_dir:path.join(testDir,'runs','first')},args:{execution_mode:'simulation',simulation_scenario:'pass',config_dir:cases,resume_mode:'fresh',dut_serial:'ACTUAL-SN',board_revision:'DEMO-REV-A',fpga_model:'DEMO-FPGA',fixture_id:'DEMO-FIXTURE-01',station_id:'DEMO-STATION-01',operator:'actual-operator'}};
 return {ctx,item,file,root,shared,legacy};
}
test('project runtime fails closed without saved shared settings, wrong scope or per-item overrides',async t=>{
 const {ctx,item,file}=fixture(t),saved=ctx.projectConfig;delete ctx.projectConfig;
 assert((await preflight(ctx)).checks.some(e=>e.code==='PROJECT.REQUIRED'));
 ctx.project='other';ctx.projectConfig=saved;assert((await preflight(ctx)).checks.some(e=>e.code==='PROJECT.CONTENT_MISMATCH'));
 ctx.project='a';fs.writeFileSync(file,JSON.stringify({...item,instruments:{}}));assert((await preflight(ctx)).checks.some(e=>e.code==='PROJECT.ITEM_OVERRIDE'));
});
test('project runtime archives raw/effective configuration, scope, numeric/UUID attempt and actual role sources',async t=>{
 const {ctx,item}=fixture(t);assert.equal((await preflight(ctx)).ok,true);const suite=await run(ctx);assert.equal(suite.product_verdict,'PASS');
 const matrix=JSON.parse(fs.readFileSync(path.join(ctx.testDir,'matrix.json'))),row=matrix.items[0],resultFile=path.resolve(ctx.layout.run_dir,'artifacts/reports',suite.results[0].result_ref),result=JSON.parse(fs.readFileSync(resultFile));
 assert.equal(result.schema_version,'wiparse.fpga-result/2');assert.equal(result.item_id,row.id);assert.equal(result.run_id,matrix.run_id);assert.equal(result.attempt,row.attempt);assert.equal(result.attempt_id,row.attempt_id);assert.equal(result.config_sha256,row.config_sha256);assert.equal(result.project_config.sha256,matrix.project_config.sha256);
 assert.deepEqual(result.scope,{project_id:'a',test_id:'test',plugin_id:'fpga-firmware-test'});assert.equal(result.identity.dut_serial,'ACTUAL-SN');assert.equal(result.identity.operator,'actual-operator');
 assert.equal(result.config_sources.identity.dut_serial,'run_parameters');assert.equal(result.config_sources.identity.operator,'run_parameters');
 assert.equal(result.config_sources.instruments.log_dongle,'project_config.instruments.log_dongle');
 const raw=JSON.parse(fs.readFileSync(path.join(path.dirname(resultFile),'raw-case.snapshot.json'))),effective=JSON.parse(fs.readFileSync(path.join(path.dirname(resultFile),'config.snapshot.json')));assert.deepEqual(raw,item);assert(!raw.instruments);assert.equal(effective.instruments.log_dongle.selector.uart_channel,0);
 assert.equal(result.effective_config_sha256,digest(effective));
 const md=fs.readFileSync(suite.summary_md,'utf8');assert(md.includes(suite.project_config.sha256));assert(md.includes('project_config.instruments.log_dongle'));
});
test('shared edits invalidate runtime and resume while unchanged frozen input ignores mutable project files',async t=>{
 const {ctx,root,shared}=fixture(t);assert.equal((await preflight(ctx)).ok,true);
 const changed=structuredClone(shared.value);changed.instruments.programmer.selector.serial='new-device';ctx.projectConfig=projectRecord(changed,{projectId:'a',testId:'test',revision:2});
 await assert.rejects(run(ctx),e=>e.code==='PROJECT.CHANGED');
 ctx.projectConfig=shared;fs.writeFileSync(path.join(root,'mutable-project.json'),JSON.stringify(changed));ctx.args.simulation_scenario='fail';assert.equal((await preflight(ctx)).ok,true);const first=await run(ctx);assert.equal(first.product_verdict,'FAIL');
 ctx.projectConfig=projectRecord(changed,{projectId:'a',testId:'test',revision:2});ctx.args.resume_mode='failed';ctx.args.resume_from=ctx.layout.run_dir;ctx.layout.run_dir=path.join(ctx.testDir,'runs','retry');
 assert((await preflight(ctx)).checks.some(e=>e.code==='RUN.RESUME_CONTENT_CHANGED'));
});
test('explicit actual identity input is never overwritten by shared applicability constraints',async t=>{
 const {ctx}=fixture(t);ctx.args.fpga_model='ACTUAL-OTHER';const result=await preflight(ctx);assert.equal(result.ok,false);assert(result.checks.some(e=>e.code==='DUT.IDENTITY_MISMATCH'));assert.equal(ctx.args.fpga_model,'ACTUAL-OTHER');
});
test('simulation-only identity fill records per-field provenance without changing supplied inputs',async t=>{
 const {ctx}=fixture(t);delete ctx.args.board_revision;delete ctx.args.dut_serial;assert.equal((await preflight(ctx)).ok,true);
 const result=await run(ctx);assert.equal(result.product_verdict,'PASS');assert.equal(result.identity.field_sources.board_revision,'project_config.simulation_constraint');assert.equal(result.identity.field_sources.dut_serial,'simulation_default');assert.equal(result.identity.field_sources.fpga_model,'run_parameters');assert.equal(ctx.args.fpga_model,'DEMO-FPGA');
});
