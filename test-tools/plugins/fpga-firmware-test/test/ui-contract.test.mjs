import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {loadSchema,loadUi} from '../../../lib/hub/schema.mjs';
import {appendReport,beginReport} from '../lib/report.mjs';
import {validateCase} from '../lib/config-loader.mjs';
import {ruleErrors} from '../lib/rule-contract.mjs';
const root=fileURLToPath(new URL('..',import.meta.url));
test('hub outputs expose wave log and shot views under item evidence',()=>{
 const manifest=JSON.parse(fs.readFileSync(path.join(root,'plugin.json'),'utf8'));
 const byId=Object.fromEntries(manifest.outputs.map(o=>[o.id,o]));
 assert.equal(byId.waves.view,'wave');
 assert.equal(byId.logs.view,'log');
 assert.equal(byId.shots.view,'external');
 assert.equal(byId.waves.dir,'{artifacts_dir}/items');
 assert.ok(byId.waves.include.includes('**/waves/**'));
 assert.ok(byId.logs.include.includes('**/logs/**'));
 assert.ok(byId.shots.include.includes('**/shots/**'));
});
test('published editor metadata loads and retains all twenty business configurations',()=>{
 const schema=loadSchema(path.join(root,'schemas/fpga-test-case.schema.json'),root);
 const ui=loadUi(path.join(root,'schemas/fpga-test-case.ui.json'),root);
 assert.equal(ui.fields.id.advanced,true);
 const cases=path.resolve(root,'../../../recipes/fpga-firmware-test/cases');
 const files=fs.readdirSync(cases).filter(f=>f.endsWith('.json'));
 assert.equal(files.length,20);
 const docs=files.map(f=>JSON.parse(fs.readFileSync(path.join(cases,f))));
 const sharedSchema=loadSchema(path.join(root,'schemas/project.schema.json'),root);
 const project=JSON.parse(fs.readFileSync(path.resolve(root,'../../../recipes/fpga-firmware-test/project.json'),'utf8'));
 assert.deepEqual(sharedSchema.errors(project),[]);
 for(const item of docs)assert.deepEqual(schema.errors(item),[],item.id);
 assert.deepEqual(ui.item_properties,['name','id','enabled','order','tags','timeout_s']);assert(!ui.groups.some(g=>g.fields.includes('dut')||g.fields.includes('instruments')||g.label==='General'));
 assert.equal(ui.widgets['log_capture.resource_role'].source_scope,'project_config');
 for(const branch of schema.schema.$defs.rule.oneOf)assert.match(branch['x-title-zh'],/[\u4e00-\u9fff]/);
 assert(!ui.widgets['verdict.rules'].summary_fields.includes('type'));
 assert.equal(ui.fields['name'].enum_labels_zh,undefined);
 assert.deepEqual(Object.keys(ui.fields['firmware.format'].enum_labels_zh).sort(),['bin','hex']);
 assert.equal(ui.widgets['verdict.rules.*.capture'].source_path,'waveform_captures');
 assert.equal(ui.widgets['verdict.rules.*.resource_role'].source_path,'instruments');
 assert.equal(ui.widgets['verdict.rules.*.evidence_ref'],undefined);
 assert(ui.required_widgets.includes('reference_selector'));
 assert.equal(ui.widgets['firmware.path'].hash_field,'firmware.sha256');
 assert.equal(schema.schema.properties.firmware.properties.sha256.readOnly,true);
 for(const reset of schema.schema.properties.reset.oneOf)for(const branch of reset.properties.release_confirmation.oneOf){assert.match(branch['x-title-zh'],/确认/);assert.equal(branch.properties.timeout_ms['x-title-zh'],'确认超时');}
 assert.equal(ui.fields['reset.release_confirmation.timeout_ms'].label_zh,'确认超时');
 assert.equal(ui.fields.timing.hidden,true);
 assert.equal(schema.schema.properties.timing.properties.anchor.const,'after_reset_release');
});
test('F13 rejects noncanonical waveform units and incompatible reset roles',()=>{
 const c=JSON.parse(fs.readFileSync(new URL('fixtures/legacy-demo-cases.json',import.meta.url)))[0];
 c.reset={method:'dc_power',power_role:'log',off_time_ms:1,release_confirmation:{type:'command_ack'}};
 assert(validateCase(c).some(e=>e.path==='/reset/power_role'));
 for(const [metric,unit]of [['frequency_hz','MHz'],['duty_cycle','%']])assert(ruleErrors({id:'r',type:'waveform_metric_range',required:true,capture:'w',channel:'CH1',metric,unit,minimum:0,maximum:1}).some(e=>e.path.endsWith('/unit')));
});
test('F15 nested report retains child limits and escapes text without inventing log links',t=>{
 const tmp=fs.mkdtempSync(path.join(process.cwd(),'fpga-report-'));t.after(()=>fs.rmSync(tmp,{recursive:true,force:true}));
 const report=path.join(tmp,'summary.md'),dir=path.join(tmp,'item');fs.mkdirSync(dir);fs.writeFileSync(path.join(dir,'result.json'),'{}');
 beginReport(report,{options:{execution_mode:'simulation'},identity:{dut_serial:'SN|<1>'}},'run');
 appendReport(report,{attempt_id:'a',execution_status:'completed',product_verdict:'PASS',limits_sha256:'hash',evidence:{artifacts:{}},results:[{id:'group',children:[{id:'child|<1>',measured:100,minimum:99,maximum:101,unit:'Hz',verdict:'PASS',source:'inline'}]}]},dir,'PLL\n测试');
 const md=fs.readFileSync(report,'utf8');assert.match(md,/child&#124;&lt;1&gt;\|100\|99\|101\|Hz\|PASS\|inline/);assert(!md.includes('serial.log'));assert.match(md,/SN&#124;&lt;1&gt;/);
});

test('report embeds waveform screenshots and retains source file links',t=>{
 const tmp=fs.mkdtempSync(path.join(process.cwd(),'fpga-inline-shot-'));t.after(()=>fs.rmSync(tmp,{recursive:true,force:true}));
 const report=path.join(tmp,'summary.md'),dir=path.join(tmp,'item');fs.mkdirSync(dir);
 appendReport(report,{attempt_id:'a',execution_status:'completed',product_verdict:'PASS',limits_sha256:'hash',evidence:{artifacts:{shot:{path:'shots/波形 (1).png'},wave:{path:'waves/capture.isf'}}},results:[]},dir,'Capture');
 const md=fs.readFileSync(report,'utf8');
 assert.match(md,/^### 波形截图$/m);
 assert.match(md,/^!\[shot\]\(item\/shots\//m);
 assert.match(md,/\[wave\]\(item\/waves\/capture.isf\)/);
 assert.equal([...md.matchAll(/^!\[/gm)].length,1);
});
