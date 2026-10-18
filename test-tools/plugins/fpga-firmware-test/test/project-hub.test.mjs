import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {execute} from '../../../hub-workspace.mjs';
import {workspace} from '../../../lib/hub/manifest.mjs';
import {ConfigStore} from '../../../lib/hub/config-store.mjs';
import {commitMigration,previewMigration,getProject} from '../../../lib/hub/project-config.mjs';
const pluginDir=fileURLToPath(new URL('..',import.meta.url));
function fixture(t){
 const root=fs.mkdtempSync(path.join(process.cwd(),'fpga-project-hub-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
 const repository=path.join(root,'cases');fs.mkdirSync(repository);fs.mkdirSync(path.join(root,'firmware'));
 fs.copyFileSync(new URL('../../../../recipes/fpga-firmware-test/firmware/demo-ip-suite.bin',import.meta.url),path.join(root,'firmware/demo-ip-suite.bin'));
 const cases=JSON.parse(fs.readFileSync(new URL('fixtures/legacy-demo-cases.json',import.meta.url)));for(const c of cases)fs.writeFileSync(path.join(repository,c.id+'.json'),JSON.stringify(c));
 const context={pluginDir,dataRoot:root,project:'a',testId:'FpgaFirmwareTest',params:{config_dir:repository}},call=(method,params={})=>execute({context,method,params,internal:true});
 const store=()=>new ConfigStore(workspace(JSON.parse(fs.readFileSync(path.join(pluginDir,'plugin.json'))),context));
 return {root,repository,cases,context,call,store};
}
test('real Hub previews and explicitly commits all 20 cases with backup and idempotence',t=>{
 const {call,cases,repository,store}=fixture(t);assert.equal(call('project.get').saved,false);
 const original=cases.map(c=>fs.readFileSync(path.join(repository,c.id+'.json')));const preview=call('project.migration.preview');assert.equal(preview.ok,true);assert.equal(preview.items.length,20);assert(preview.role_mapping.length>20);assert.equal(preview.source_differences.length,1);
 for(let i=0;i<cases.length;i++)assert.deepEqual(fs.readFileSync(path.join(repository,cases[i].id+'.json')),original[i]);
 assert.equal(call('project.migration.commit',{plan_sha256:preview.plan_sha256}).migrated,20);
 const project=call('project.get');assert.equal(project.saved,true);assert.equal(project.value.instruments.log_com.selector.port,'COM3');assert.equal(project.value.instruments.log_dongle.selector.uart_channel,0);
 const list=call('config.list',{limit:500});assert.equal(list.items.length,20);assert(list.items.every(row=>!row.errors.length));
 const saved=store(),backup=JSON.parse(fs.readFileSync(path.join(path.dirname(saved.s.projectFile),'backups','migration-'+preview.plan_sha256+'.json')));assert.equal(backup.items.length,20);for(let i=0;i<cases.length;i++)assert.deepEqual(Buffer.from(backup.items[i].bytes,'base64'),original[i]);
 const again=call('project.migration.preview');assert.equal(again.items.length,0);assert.equal(call('project.migration.commit',{plan_sha256:again.plan_sha256}).migrated,0);assert.equal(call('project.get').revision,project.revision);
});
test('Hub conflict or stale preview cannot partially migrate user data',t=>{
 const {call,cases,repository}=fixture(t);const preview=call('project.migration.preview'),file=path.join(repository,cases[0].id+'.json');
 const altered=structuredClone(cases[0]);altered.name='changed after preview';fs.writeFileSync(file,JSON.stringify(altered));assert.throws(()=>call('project.migration.commit',{plan_sha256:preview.plan_sha256}),e=>e.code==='CONFIG.REVISION_CONFLICT');
 altered.instruments.programmer.selector.serial='conflict';fs.writeFileSync(file,JSON.stringify(altered));const conflict=call('project.migration.preview');assert.equal(conflict.ok,false);assert.throws(()=>call('project.migration.commit',{plan_sha256:conflict.plan_sha256}),e=>e.code==='PROJECT.MIGRATION_CONFLICT');assert.equal(call('project.get').saved,false);assert.equal(JSON.parse(fs.readFileSync(file)).schema_version,'wiparse.fpga-test-case/1');
});
test('migration replacement failure rolls back and interrupted rollback recovers through host journal',t=>{
 for(const recovery of [false,true]){
  const {store,cases,repository}=fixture(t),s=store(),plan=previewMigration(s),original=cases.map(c=>fs.readFileSync(path.join(repository,c.id+'.json')));
  s.fault=where=>{if(where==='after_replace')throw Object.assign(Error('injected disk failure'),{code:'ENOSPC'});};
  if(recovery)s.rollback=()=>{throw Object.assign(Error('rollback storage unavailable'),{code:'EIO'});};
  assert.throws(()=>commitMigration(s,{plan_sha256:plan.plan_sha256}),e=>e.code===(recovery?'CONFIG.RECOVERY_REQUIRED':'ENOSPC'));
  const fresh=store();fresh.withLock(()=>{assert.equal(getProject(fresh).saved,false);});
  for(let i=0;i<cases.length;i++)assert.deepEqual(fs.readFileSync(path.join(repository,cases[i].id+'.json')),original[i]);
  const next=previewMigration(fresh);assert.equal(commitMigration(fresh,{plan_sha256:next.plan_sha256}).migrated,20);
 }
});
