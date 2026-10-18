import fs from 'node:fs';
import path from 'node:path';
import {hash} from '../../lib/common.mjs';
import {canonical} from '../../lib/project-config.mjs';
import {planProjectMigration} from '../../lib/project-migration.mjs';

export function projectRecord(value,{testId='test',projectId='test',revision=1}={}){
 const body={schema_version:'wiparse.project-config/1',project_id:projectId,test_id:testId,plugin_id:'fpga-firmware-test',revision,schema_hash:'a'.repeat(64),value:structuredClone(value)};
 return {...body,sha256:hash(canonical(body))};
}
// Explicit test-fixture preparation. Production migration remains host-owned and transactional.
export function prepareProjectFixture(ctx){
 const dir=ctx.args.config_dir,files=fs.readdirSync(dir).filter(f=>f.endsWith('.json')&&!f.startsWith('_'));
 const cases=files.map(f=>JSON.parse(fs.readFileSync(path.join(dir,f),'utf8')));
 if(!cases.some(c=>c.schema_version==='wiparse.fpga-test-case/1'))return;
 const plan=planProjectMigration({cases,projectConfig:ctx.projectConfig??null});
 if(!plan.ok)throw Error(`Invalid test fixture migration: ${JSON.stringify(plan.conflicts)}`);
 const same=ctx.projectConfig&&canonical(ctx.projectConfig.value)===canonical(plan.project_value);
 if(!same)ctx.projectConfig=projectRecord(plan.project_value,{testId:path.basename(ctx.layout?.test_dir??ctx.testDir),projectId:ctx.project??'test',revision:(ctx.projectConfig?.revision??0)+1});
 for(const item of plan.items){const index=cases.findIndex(c=>c.id===item.id);fs.writeFileSync(path.join(dir,files[index]),JSON.stringify(item.value));}
}
export function caseDocument(legacy){const value=structuredClone(legacy);value.schema_version='wiparse.fpga-test-case/2';delete value.dut;delete value.instruments;if(value.log_capture.enabled)value.log_capture.resource_role='log';return value;}
