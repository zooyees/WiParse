import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {execute} from '../../../hub-workspace.mjs';
import {discover} from '../lib/config-loader.mjs';

const pluginDir=fileURLToPath(new URL('..',import.meta.url));

function contextFor(t){
 const dataRoot=fs.mkdtempSync(path.join(os.tmpdir(),'fpga-draft-save-'));
 t.after(()=>fs.rmSync(dataRoot,{recursive:true,force:true}));
 const cases=path.join(dataRoot,'cases');
 fs.mkdirSync(cases);
 return {dataRoot,cases,context:{pluginDir,dataRoot,project:'demo',testId:'FpgaFirmwareTest',params:{config_dir:cases}}};
}

test('a new FPGA case can be saved before firmware, rules, and project roles are complete',t=>{
 const {cases,context}=contextFor(t);
 const draft=execute({context,method:'config.create',params:{config_id:'MT58XX'},internal:true});
 assert.equal(draft.value.name,'新测试项');
 assert.deepEqual(draft.errors,[]);
 const onScreen=structuredClone(draft.value);
 onScreen.firmware.sha256='';
 onScreen.reset.power_role='';
 onScreen.verdict.rules[0].id='';
 onScreen.verdict.rules[0].text='';
 const saved=execute({context,method:'config.save',params:{config_id:'MT58XX',value:onScreen,expected_revision:0,expected_sha256:''},internal:true});
 assert.equal(saved.valid,true);
 assert.equal(saved.config_id,'MT58XX');
 assert.ok(saved.warnings.length>0);
 assert.equal(fs.existsSync(path.join(cases,'MT58XX.json')),true);
 const found=discover(cases);
 assert.equal(found.items.length,0);
 assert.ok(found.errors.length>0);
 const incomplete=structuredClone(onScreen);
 incomplete.firmware.sha256='not-a-hash';
 incomplete.verdict.rules[0].text='';
 const stored=execute({context,method:'config.save',params:{config_id:'MT58XX',value:incomplete,expected_revision:saved.revision,expected_sha256:saved.sha256},internal:true});
 assert.equal(stored.valid,true);
 assert.ok(stored.warnings.some(w=>w.path==='/firmware/sha256'));
 assert.throws(()=>execute({context,method:'config.save',params:{config_id:'MT58XX',value:{...incomplete,name:''},expected_revision:stored.revision,expected_sha256:stored.sha256},internal:true}),e=>e.code==='SCHEMA.VALIDATION_FAILED');
});
