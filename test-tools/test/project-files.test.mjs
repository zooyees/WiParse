import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {ConfigStore} from '../lib/hub/config-store.mjs';
import {workspace} from '../lib/hub/manifest.mjs';
import {getProject, saveProject} from '../lib/hub/project-config.mjs';
import {activateProjectFile, exportProjectFile, importProjectFile, projectFileSnapshot} from '../lib/hub/project-files.mjs';
import {readJson} from '../lib/hub/common.mjs';

const demo = fileURLToPath(new URL('../plugins/hub-workspace-demo', import.meta.url));

function setup(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'project-files-'));
  t.after(() => fs.rmSync(root, {recursive: true, force: true}));
  const dir = path.join(root, 'plugin');
  fs.cpSync(demo, dir, {recursive: true});
  const plugin = readJson(path.join(dir, 'plugin.json'));
  plugin.ui.config_editor.repository = '{data_root}/cases';
  plugin.ui.project_files = {directory: '{data_root}/project-files', max_cases: 500};
  plugin.ui.project_config = {protocol: 'wiparse.project-config/1', schema: 'project.schema.json'};
  fs.writeFileSync(path.join(dir, 'plugin.json'), JSON.stringify(plugin));
  fs.writeFileSync(path.join(dir, 'config.schema.json'), JSON.stringify({type: 'object', additionalProperties: true}));
  fs.writeFileSync(path.join(dir, 'project.schema.json'), JSON.stringify({type: 'object', properties: {name: {type: 'string'}}, required: ['name'], additionalProperties: true}));
  fs.writeFileSync(path.join(dir, 'index.mjs'), 'export function configure(){return {lifecycle:"configure",ok:true,errors:[],warnings:[]};}\nexport function configureProject(){return {ok:true,errors:[],warnings:[]};}\n');
  const dataRoot = path.join(root, 'data');
  const store = new ConfigStore(workspace(plugin, {pluginDir: dir, dataRoot, project: 'lot', testId: 'Cases'}));
  const current = getProject(store);
  saveProject(store, {value: {name: 'Demo board'}, expected_revision: current.revision, expected_sha256: current.sha256});
  return {store, dataRoot};
}
function writeCase(store, caseId, name) {
  const draft = store.draft(caseId);
  draft.value.name = name;
  return store.save({config_id: caseId, value: draft.value, expected_revision: 0, expected_sha256: ''});
}
const ids = store => store.list().items.map(item => item.id).sort();

test('switching a project file replaces the case list and switching back restores it', t => {
  const {store, dataRoot} = setup(t);
  writeCase(store, 'alpha', 'Alpha');
  writeCase(store, 'beta', 'Beta');
  const opened = projectFileSnapshot(store);
  assert.equal(opened.active, 'demo');
  assert.deepEqual(ids(store), ['alpha', 'beta']);
  const other = {
    schema_version: 'wiparse.fpga-project-file/1',
    id: 'other',
    name: '其他项目',
    project: {name: 'Other board'},
    cases: [{id: 'only', name: 'Only case'}],
  };
  fs.mkdirSync(path.join(dataRoot, 'project-files'), {recursive: true});
  fs.writeFileSync(path.join(dataRoot, 'project-files', 'other.json'), JSON.stringify(other));
  const switched = activateProjectFile(store, {id: 'other'});
  assert.equal(switched.cases, 1);
  assert.deepEqual(ids(store), ['only']);
  assert.equal(getProject(store).value.name, 'Other board');
  activateProjectFile(store, {id: 'demo'});
  assert.deepEqual(ids(store), ['alpha', 'beta']);
  assert.equal(getProject(store).value.name, 'Demo board');
  assert.equal(projectFileSnapshot(store).active, 'demo');
});

test('a project file cannot contain more than 500 cases', t => {
  const {store, dataRoot} = setup(t);
  writeCase(store, 'alpha', 'Alpha');
  projectFileSnapshot(store);
  const cases = Array.from({length: 501}, (_, i) => ({id: `c${i}`, name: `Case ${i}`}));
  fs.writeFileSync(path.join(dataRoot, 'project-files', 'huge.json'), JSON.stringify({
    schema_version: 'wiparse.fpga-project-file/1', id: 'huge', name: 'Too many', project: {name: 'Huge'}, cases,
  }));
  assert.throws(() => activateProjectFile(store, {id: 'huge'}), /REPOSITORY_LIMIT/);
  assert.deepEqual(ids(store), ['alpha']);
});

test('import and export round-trip one project file', t => {
  const {store} = setup(t);
  writeCase(store, 'alpha', 'Alpha');
  const outside = path.join(os.tmpdir(), `fpga-project-${Date.now()}.json`);
  t.after(() => fs.rmSync(outside, {force: true}));
  const incoming = {
    schema_version: 'wiparse.fpga-project-file/1',
    id: 'imported',
    name: '导入项目',
    project: {name: 'Imported board'},
    cases: [{id: 'imported_case', name: 'Imported'}],
  };
  fs.writeFileSync(outside, JSON.stringify(incoming));
  const loaded = importProjectFile(store, {file: outside}, {internal: true});
  assert.equal(loaded.id, 'imported');
  assert.deepEqual(ids(store), ['imported_case']);
  const exported = path.join(os.tmpdir(), `fpga-project-out-${Date.now()}.json`);
  t.after(() => fs.rmSync(exported, {force: true}));
  exportProjectFile(store, {file: exported}, {internal: true});
  const doc = readJson(exported);
  assert.equal(doc.id, 'imported');
  assert.equal(doc.cases.length, 1);
  assert.equal(doc.cases[0].id, 'imported_case');
  assert.equal(doc.project.name, 'Imported board');
});
