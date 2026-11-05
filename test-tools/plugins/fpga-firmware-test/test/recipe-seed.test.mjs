import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import run,{preflight} from '../index.mjs';
import {validateProjectValue} from '../lib/project-config.mjs';
import {planProjectMigration} from '../lib/project-migration.mjs';
import {loadSchema} from '../../../lib/hub/schema.mjs';
import {projectRecord} from './fixtures/project-fixture.mjs';

const pluginRoot=fileURLToPath(new URL('..',import.meta.url));
const recipeRoot=path.resolve(pluginRoot,'../../../recipes/fpga-firmware-test');
const casesDir=path.join(recipeRoot,'cases');
const firmware=path.join(recipeRoot,'firmware/demo-ip-suite.bin');

function savedContext(t,configDir,repositoryRoot,projectValue,args={}){
 const testDir=fs.mkdtempSync(path.join(process.cwd(),'fpga-seed-'));
 t.after(()=>fs.rmSync(testDir,{recursive:true,force:true}));
 const testId=path.basename(testDir);
 return {project:'demo',projectConfig:projectRecord(projectValue,{projectId:'demo',testId}),repositoryRoot,testDir,layout:{run_dir:path.join(testDir,'runs','sim')},args:{execution_mode:'simulation',simulation_scenario:'pass',config_dir:configDir,resume_mode:'fresh',dut_serial:'DEMO-SN-0001',board_revision:'DEMO-REV-A',fpga_model:'DEMO-FPGA',fixture_id:'DEMO-FIXTURE-01',station_id:'DEMO-STATION-01',operator:'demo',...args}};
}
function stageLegacy(t,cases){
 const root=fs.mkdtempSync(path.join(process.cwd(),'fpga-legacy-'));
 t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
 const dir=path.join(root,'cases');
 fs.mkdirSync(dir);fs.mkdirSync(path.join(root,'firmware'));
 fs.copyFileSync(firmware,path.join(root,'firmware/demo-ip-suite.bin'));
 for(const item of cases)fs.writeFileSync(path.join(dir,`${item.id}.json`),JSON.stringify(item));
 return {root,dir};
}

test('recipe cases are schema/2 and the shared project keeps every distinct role',()=>{
 const manifest=JSON.parse(fs.readFileSync(path.join(pluginRoot,'plugin.json'),'utf8'));
 assert.equal(manifest.version,'0.5.5');
 assert.equal(manifest.name_zh,'FPGA验证');
 assert.equal(manifest.ui.project_files.directory,'{config_dir}/../projects');
 assert.equal(manifest.ui.project_files.max_cases,500);
 assert.equal(manifest.host_adapter,'wiparse.fpga-host/1');
 assert.equal(manifest.ui.project_config.seed,'{config_dir}/../project.json');
 assert.equal(manifest.params.find(p=>p.name==='execution_mode').default,'hardware');
 const files=fs.readdirSync(casesDir).filter(f=>f.endsWith('.json')).sort();
 assert.equal(files.length,20);
 const cases=files.map(file=>({file,value:JSON.parse(fs.readFileSync(path.join(casesDir,file),'utf8'))}));
 for(const {file,value} of cases){
  assert.equal(value.schema_version,'wiparse.fpga-test-case/2',file);
  assert.equal(file,`${value.id}.json`);
  assert.equal('dut' in value,false,file);
  assert.equal('instruments' in value,false,file);
  assert.equal(value.firmware.download,false,file);
 }
 const project=JSON.parse(fs.readFileSync(path.join(recipeRoot,'project.json'),'utf8'));
 assert.deepEqual(validateProjectValue(project),[]);
 const schema=loadSchema(path.join(pluginRoot,'schemas/project.schema.json'),pluginRoot);
 assert.deepEqual(schema.errors(project),[]);
 assert.deepEqual(Object.keys(project.instruments),['programmer','scope','log_dongle','log_com','power']);
 const analog=cases.filter(c=>c.value.tags.includes('analog-scope')).map(c=>c.value.id);
 const selftest=cases.filter(c=>c.value.tags.includes('log-selftest')).map(c=>c.value.id);
 assert.deepEqual(analog,['fpga_01_pll_clock','fpga_09_pwm']);
 assert.equal(selftest.length,18);
 const packed=JSON.parse(fs.readFileSync(path.join(recipeRoot,'projects/demo.json'),'utf8'));
 assert.equal(packed.schema_version,'wiparse.fpga-project-file/1');
 assert.equal(packed.id,'demo');
 assert.ok(packed.cases.length<=500);
 assert.deepEqual(packed.cases.map(item=>item.id).sort(),files.map(file=>file.slice(0,-5)).sort());
 assert.equal(packed.project.name,project.name);
 for(const {value} of cases){
  if(value.tags.includes('analog-scope'))assert.equal(value.waveform_captures[0].source_waveform.enabled,true,value.id);
  if(value.tags.includes('log-selftest'))assert.equal(value.waveform_captures[0].source_waveform.enabled,false,value.id);
 }
 const dongle=cases.filter(c=>c.value.log_capture.resource_role==='log_dongle').map(c=>c.value.id);
 const com=cases.filter(c=>c.value.log_capture.resource_role==='log_com').map(c=>c.value.id);
 const dc=cases.filter(c=>c.value.reset.method==='dc_power').map(c=>c.value.id);
 assert.deepEqual(dongle,['fpga_01_pll_clock','fpga_03_gpio','fpga_05_spi_master','fpga_07_i2c_master','fpga_09_pwm','fpga_11_watchdog','fpga_13_async_fifo','fpga_15_ddr_controller','fpga_17_axi_stream','fpga_19_ethernet_mac']);
 assert.deepEqual(com,['fpga_02_reset_controller','fpga_04_uart','fpga_06_spi_slave','fpga_08_i2c_slave','fpga_10_timer_counter','fpga_12_sync_fifo','fpga_14_block_ram','fpga_16_axi_lite','fpga_18_dma','fpga_20_crc32']);
 assert.deepEqual(dc,['fpga_02_reset_controller','fpga_06_spi_slave','fpga_10_timer_counter','fpga_14_block_ram','fpga_18_dma']);
 for(const {value} of cases){
  if(value.reset.method==='dc_power')assert.equal(project.instruments[value.reset.power_role].kind,'dc_source');
  if(value.reset.method==='wiparse_dongle')assert.equal(project.instruments[value.reset.dongle_role].kind,'debug_probe');
  assert.ok(project.instruments[value.log_capture.resource_role]);
 }
 assert.equal(project.instruments.log_dongle.selector.dongle_serial,project.instruments.programmer.selector.serial);
});

test('saved schema/2 demo preflight uses simulation and does not ask for a hardware adapter',async t=>{
 const project=JSON.parse(fs.readFileSync(path.join(recipeRoot,'project.json'),'utf8'));
 const ctx=savedContext(t,casesDir,recipeRoot,project);
 assert.equal(ctx.fpgaAdapter,undefined);
 const result=await preflight(ctx);
 assert.equal(result.ok,true,JSON.stringify(result.checks));
 assert.equal(result.simulation_only,true);
 assert.equal(result.industrial_approved,false);
 assert.equal(result.checks.some(c=>c.code==='HOST.ADAPTER_REQUIRED'),false);
 assert.equal(ctx.fpgaAdapter.simulation_only,true);
 assert.equal(ctx.fpgaAdapter.version,'wiparse.fpga-host/1');
});

test('simulation run of fpga_01_pll_clock records simulation_only evidence',{timeout:120000},async t=>{
 const project=JSON.parse(fs.readFileSync(path.join(recipeRoot,'project.json'),'utf8'));
 const ctx=savedContext(t,casesDir,recipeRoot,project,{selected_ids:'fpga_01_pll_clock'});
 const ready=await preflight(ctx);
 assert.equal(ready.ok,true,JSON.stringify(ready.checks));
 const suite=await run(ctx);
 assert.equal(suite.simulation_only,true);
 assert.equal(suite.industrial_approved,false);
 assert.equal(suite.execution_status,'completed');
 assert.equal(suite.product_verdict,'PASS');
 assert.equal(suite.results.length,1);
 assert.equal(suite.results[0].item_id,'fpga_01_pll_clock');
 const report=fs.readFileSync(suite.summary_md,'utf8');
 assert.match(report,/模拟演示/);
 const result=JSON.parse(fs.readFileSync(path.resolve(path.dirname(suite.summary_md),suite.results[0].result_ref),'utf8'));
 assert.equal(result.evidence.log.simulation_only,true);
 assert.equal(result.industrial_approved,false);
});

test('schema/1 fixture still previews and direct preflight fails',async t=>{
 const cases=JSON.parse(fs.readFileSync(new URL('fixtures/legacy-demo-cases.json',import.meta.url)));
 const before=JSON.stringify(cases);
 const plan=planProjectMigration({cases,projectConfig:null});
 assert.equal(JSON.stringify(cases),before);
 assert.equal(plan.ok,true,JSON.stringify(plan.conflicts));
 assert.equal(plan.items.length,20);
 assert.equal(plan.source_differences.length,1);
 assert.ok(plan.project_value?.instruments.log_dongle);
 assert.ok(plan.project_value?.instruments.log_com);
 assert.ok(plan.project_value?.instruments.power);
 const staged=stageLegacy(t,cases);
 const ctx=savedContext(t,staged.dir,staged.root,plan.project_value);
 const result=await preflight(ctx);
 assert.equal(result.ok,false);
 assert.equal(result.checks.some(c=>c.code==='PROJECT.MIGRATION_REQUIRED'),true,JSON.stringify(result.checks));
});

test('firmware download still fails preflight',async t=>{
 const project=JSON.parse(fs.readFileSync(path.join(recipeRoot,'project.json'),'utf8'));
 const item=JSON.parse(fs.readFileSync(path.join(casesDir,'fpga_01_pll_clock.json'),'utf8'));
 item.firmware.download=true;
 const staged=stageLegacy(t,[item]);
 const ctx=savedContext(t,staged.dir,staged.root,project,{selected_ids:item.id});
 const result=await preflight(ctx);
 assert.equal(result.ok,false);
 assert.equal(result.checks.some(c=>c.code==='FEATURE.FIRMWARE_DOWNLOAD_NOT_IMPLEMENTED'),true,JSON.stringify(result.checks));
 assert.equal(JSON.parse(fs.readFileSync(path.join(casesDir,'fpga_01_pll_clock.json'),'utf8')).firmware.download,false);
});
