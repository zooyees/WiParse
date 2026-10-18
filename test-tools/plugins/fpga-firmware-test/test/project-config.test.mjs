import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import {hash} from '../lib/common.mjs';
import {canonical,checkedProject,validateProjectValue,resolveProjectCase} from '../lib/project-config.mjs';
function fixture(){
 const legacy=JSON.parse(fs.readFileSync(new URL('../../../../recipes/fpga-firmware-test/cases/fpga_01_pll_clock.json',import.meta.url)));
 const project={schema_version:'wiparse.project-config/1',project_id:'project-a',test_id:'FpgaFirmwareTest',plugin_id:'fpga-firmware-test',revision:1,schema_hash:'a'.repeat(64),value:{schema_version:'wiparse.fpga-project/1',name:'工程A',description:'',dut:legacy.dut,instruments:legacy.instruments}};
 const item=structuredClone(legacy);delete item.dut;delete item.instruments;item.schema_version='wiparse.fpga-test-case/2';item.log_capture.resource_role='log';
 return {item,project:seal(project)};
}
function seal(value){const {sha256,...body}=value;return {...body,sha256:hash(canonical(body))};}
test('project snapshot requires saved valid scope and canonical hash',()=>{
 const {project}=fixture();assert.equal(checkedProject(project,{projectId:'project-a',testId:'FpgaFirmwareTest'}).sha256,project.sha256);
 assert.throws(()=>checkedProject({...project,revision:0}),/PROJECT.REQUIRED/);
 assert.throws(()=>checkedProject(project,{projectId:'project-b'}),/PROJECT.CONTENT_MISMATCH/);
 assert.throws(()=>checkedProject(project,{testId:'another-test'}),/PROJECT.CONTENT_MISMATCH/);
 assert.throws(()=>checkedProject({...project,value:{...project.value,name:'tampered'}}),/PROJECT.CONTENT_MISMATCH/);
});
test('effective case uses frozen project without mutating raw case or actual identity inputs',()=>{
 const {item,project}=fixture(),before=JSON.stringify({item,project});const result=resolveProjectCase(item,project);
 assert.deepEqual(result.value.dut,project.value.dut);assert.deepEqual(result.project_config,{revision:1,sha256:project.sha256,schema_hash:project.schema_hash});
 assert.equal(result.sources.identity,'run_parameters');assert.equal(result.value.name,item.name);assert.equal(result.value.timeout_s,item.timeout_s);assert.equal(JSON.stringify({item,project}),before);
 project.value.dut.board_revision='later';assert.equal(result.value.dut.board_revision,'DEMO-REV-A');
});
test('case binding overrides and unresolved roles fail explicitly',()=>{
 const {item,project}=fixture();assert.throws(()=>resolveProjectCase({...item,instruments:{}},project),/PROJECT.ITEM_OVERRIDE/);
 item.log_capture.resource_role='missing';assert.throws(()=>resolveProjectCase(item,project),/PROJECT.ROLE_MISSING/);
 delete item.log_capture.resource_role;assert.throws(()=>resolveProjectCase(item,project),/PROJECT.ROLE_REQUIRED/);
 item.schema_version='wiparse.fpga-test-case/1';assert.throws(()=>resolveProjectCase(item,project),/PROJECT.MIGRATION_REQUIRED/);
});
test('changed shared binding changes effective identity and does not reuse old project hash',()=>{
 const {item,project}=fixture(),old=resolveProjectCase(item,project);project.value.instruments.programmer.selector.serial='other';project.revision++;const next=resolveProjectCase(item,seal(project));
 assert.notEqual(next.effective_sha256,old.effective_sha256);assert.notEqual(next.project_config.sha256,old.project_config.sha256);
});
test('shared role validation rejects unstable IDs and invalid UART channels',()=>{
 const {project}=fixture();project.value.instruments.log.selector.uart_channel=-1;project.value.instruments.log.selector.device_id=7;
 const errors=validateProjectValue(project.value);assert(errors.some(e=>e.message.includes('device_id')));assert(errors.some(e=>e.message.includes('通道')));
});
