import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import {hash} from '../lib/common.mjs';
import {canonical,resolveProjectCase} from '../lib/project-config.mjs';
import {planProjectMigration} from '../lib/project-migration.mjs';
const fixture=()=>JSON.parse(fs.readFileSync(new URL('fixtures/legacy-demo-cases.json',import.meta.url)));
function saved(value){const body={schema_version:'wiparse.project-config/1',project_id:'a',test_id:'t',plugin_id:'fpga-firmware-test',revision:1,schema_hash:'a'.repeat(64),value};return {...body,sha256:hash(canonical(body))};}
test('20-item preview preserves every name, binding, log transport, reset and timeout without mutation',()=>{
 const cases=fixture(),before=JSON.stringify(cases),plan=planProjectMigration({cases,projectConfig:null});assert.equal(plan.ok,true,JSON.stringify(plan.conflicts));assert.equal(JSON.stringify(cases),before);assert.equal(plan.items.length,20);assert.equal(plan.source_differences.length,1);
 const shared=saved(plan.project_value);let com=0,dongle=0,dc=0;
 for(const item of plan.items){const old=cases.find(c=>c.id===item.id),effective=resolveProjectCase(item.value,shared).value;assert.equal(item.value.name,old.name);assert.equal(item.value.timeout_s,old.timeout_s);assert.deepEqual(item.value.reset,old.reset);assert(!('dut'in item.value));assert(!('instruments'in item.value));assert.deepEqual(effective.dut,old.dut);assert.deepEqual(effective.instruments[item.value.log_capture.resource_role],old.instruments.log);com+=item.value.log_capture.resource_role==='log_com';dongle+=item.value.log_capture.resource_role==='log_dongle';dc+=item.value.reset.method==='dc_power';}
 assert.deepEqual([com,dongle,dc],[10,10,5]);assert.equal(plan.project_value.instruments.power.selector.serial,'DEMO-POWER-001');
});
test('migration refuses incompatible DUT constraints and same-role device differences',()=>{
 for(const mutate of [c=>c[1].dut.board_revision='B',c=>c[1].instruments.programmer.selector.serial='different',c=>c[3].instruments.log.selector.port='COM9']){const cases=fixture();mutate(cases);const plan=planProjectMigration({cases,projectConfig:null});assert.equal(plan.ok,false);assert(plan.conflicts.length>0);assert.equal(plan.project_value,null);assert.deepEqual(plan.items,[]);}
});
test('migration never overwrites a conflicting saved project binding',()=>{
 const cases=fixture(),first=planProjectMigration({cases,projectConfig:null}),record=saved(first.project_value);record.value.instruments.log_com.selector.port='COM99';const edited=saved(record.value),before=JSON.stringify(edited);const plan=planProjectMigration({cases,projectConfig:edited});assert.equal(plan.ok,false);assert.equal(JSON.stringify(edited),before);assert(plan.conflicts.some(e=>e.path==='/instruments/log_com'));
});
test('migration is idempotent after explicit host commit and rejects missing saved shared data',()=>{
 const plan=planProjectMigration({cases:fixture(),projectConfig:null}),record=saved(plan.project_value),cases=plan.items.map(i=>i.value);const again=planProjectMigration({cases,projectConfig:record});assert.equal(again.ok,true);assert.deepEqual(again.items,[]);assert.deepEqual(again.project_value,record.value);assert.equal(planProjectMigration({cases,projectConfig:null}).ok,false);
});
test('migration rejects unresolved references, unsupported versions and duplicate IDs',()=>{
 const cases=fixture();cases[0].reset.dongle_role='missing';assert.equal(planProjectMigration({cases,projectConfig:null}).ok,false);
 cases[0].schema_version='unknown';assert.equal(planProjectMigration({cases,projectConfig:null}).ok,false);
 assert.equal(planProjectMigration({cases:[fixture()[0],fixture()[0]],projectConfig:null}).ok,false);
});
