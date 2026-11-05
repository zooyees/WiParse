import fs from 'node:fs';
import path from 'node:path';
import {atomicWrite, canonical, fail, id, inside, jail, readJson} from './common.mjs';
import {getProject, projectRecord, validateProject} from './project-config.mjs';

const SCHEMA = 'wiparse.fpga-project-file/1';

function catalogDir(store) {
  if (!store.s.projectFilesDir) fail('PROJECT.UNSUPPORTED');
  const dir = jail(store.s.projectFilesDir, store.s.roots);
  fs.mkdirSync(dir, {recursive: true});
  return dir;
}
function pointerFile(store) {
  return path.join(store.s.editorRoot, 'active-project.json');
}
function readPointer(store) {
  const file = pointerFile(store);
  if (!fs.existsSync(file)) return '';
  try {
    const parsed = readJson(jail(file, store.s.roots), 4096);
    return typeof parsed.id === 'string' ? parsed.id : '';
  } catch {
    return '';
  }
}
function writePointer(store, projectId) {
  atomicWrite(jail(pointerFile(store), store.s.roots), JSON.stringify({id: projectId}) + '\n');
}
function fileFor(dir, projectId) {
  return path.join(dir, id(projectId) + '.json');
}
function projectValueOf(doc) {
  if (doc.project && typeof doc.project === 'object' && !Array.isArray(doc.project)) return structuredClone(doc.project);
  if (doc.dut && typeof doc.dut === 'object' && doc.instruments && typeof doc.instruments === 'object') {
    return {
      schema_version: 'wiparse.fpga-project/1',
      name: doc.name,
      description: typeof doc.description === 'string' ? doc.description : '',
      dut: doc.dut,
      instruments: doc.instruments,
    };
  }
  return null;
}
function normalize(doc, projectId) {
  if (!doc || typeof doc !== 'object' || Array.isArray(doc) || doc.schema_version !== SCHEMA) fail('CONFIG.IMPORT_REJECTED', 'Not a project configuration file');
  if (doc.id !== projectId) fail('CONFIG.IMPORT_REJECTED', 'Project id and filename differ');
  id(projectId);
  if (typeof doc.name !== 'string' || !doc.name.trim() || doc.name.trim().length > 128) fail('CONFIG.IMPORT_REJECTED', 'Project name is required');
  if (!Array.isArray(doc.cases)) fail('CONFIG.IMPORT_REJECTED', 'Cases must be an array');
  const project = projectValueOf(doc);
  return {
    schema_version: SCHEMA,
    id: projectId,
    name: doc.name.trim(),
    description: typeof doc.description === 'string' ? doc.description.slice(0, 2048) : '',
    ...(project ? {project} : {}),
    cases: doc.cases.map(item => structuredClone(item)),
  };
}
function readDoc(store, projectId) {
  const dir = catalogDir(store);
  const file = jail(fileFor(dir, projectId), [dir]);
  if (!fs.existsSync(file)) fail('CONFIG.NOT_FOUND', projectId);
  return normalize(readJson(file, store.s.maxTotalBytes + 1048576), projectId);
}
function writeDoc(store, doc) {
  const dir = catalogDir(store);
  atomicWrite(jail(fileFor(dir, doc.id), [dir]), JSON.stringify(doc, null, 2) + '\n');
}
function listDocs(store) {
  const dir = catalogDir(store);
  const items = [];
  for (const name of fs.readdirSync(dir)) {
    if (!name.endsWith('.json') || name.startsWith('.') || name.startsWith('_')) continue;
    const projectId = name.slice(0, -5);
    try {
      const doc = readDoc(store, projectId);
      items.push({id: doc.id, name: doc.name, cases: doc.cases.length});
    } catch {
      // A stray file must not hide the projects that do parse.
    }
  }
  items.sort((a, b) => a.name.localeCompare(b.name, 'en') || a.id.localeCompare(b.id));
  return items;
}
function documentFromRepository(store, projectId) {
  const project = store.projectDefinition ? getProject(store) : null;
  const value = project?.value && typeof project.value === 'object' && !Array.isArray(project.value) ? structuredClone(project.value) : null;
  const name = typeof value?.name === 'string' && value.name.trim() ? value.name.trim() : projectId;
  return {
    schema_version: SCHEMA,
    id: id(projectId),
    name: name.slice(0, 128),
    description: typeof value?.description === 'string' ? value.description : '',
    ...(value ? {project: value} : {}),
    cases: store.scan().map(row => structuredClone(row.value)),
  };
}
function persistActive(store) {
  const active = readPointer(store);
  if (!active) return;
  const dir = catalogDir(store);
  if (!fs.existsSync(fileFor(dir, active))) return;
  writeDoc(store, documentFromRepository(store, active));
}
function checkedCases(store, cases) {
  const limit = store.s.projectFileLimit || store.s.maxItems;
  if (cases.length > limit) fail('CONFIG.REPOSITORY_LIMIT', `A project holds at most ${limit} cases`);
  const seen = new Set();
  const values = [];
  for (const raw of cases) {
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) fail('CONFIG.IMPORT_REJECTED', 'Each case must be an object');
    const value = structuredClone(raw);
    id(value.id);
    if (seen.has(value.id.toLowerCase())) fail('CONFIG.ID_CONFLICT', value.id);
    seen.add(value.id.toLowerCase());
    values.push(value);
  }
  return {values, seen};
}
function materialize(store, doc) {
  const {values, seen} = checkedCases(store, doc.cases);
  const rows = store.scan();
  const byId = new Map(rows.map(row => [row.id, row]));
  const entries = [];
  let total = 0;
  for (const value of values) {
    const bytes = Buffer.from(JSON.stringify(value, null, 2) + '\n');
    if (bytes.length > store.s.maxItemBytes) fail('CONFIG.REPOSITORY_LIMIT');
    const old = byId.get(value.id);
    if (old && canonical(old.value) === canonical(value)) total += old.bytes.length;
    else {
      total += bytes.length;
      entries.push({file: store.file(value.id), bytes});
    }
  }
  if (total > store.s.maxTotalBytes) fail('CONFIG.REPOSITORY_LIMIT');
  for (const row of rows) if (!seen.has(row.id.toLowerCase())) entries.push({file: store.file(row.id), bytes: null});
  const project = doc.project;
  if (store.projectDefinition) {
    if (!project) fail('CONFIG.IMPORT_REJECTED', 'Project settings are required');
    validateProject(store, project);
    const old = getProject(store);
    const same = old.saved && canonical(old.value) === canonical(project);
    if (!same) {
      const record = projectRecord(store, project, (old.saved ? old.revision : 0) + 1);
      const bytes = Buffer.from(JSON.stringify(record, null, 2) + '\n');
      if (bytes.length > 1048576) fail('CONFIG.REPOSITORY_LIMIT');
      entries.push({file: store.s.projectFile, bytes});
    }
  }
  store.assertUnlocked(values.map(value => value.id).concat(rows.map(row => row.id)));
  if (entries.length) {
    store.commit(entries, [{operation: 'project.file.activate', config_id: doc.id, item_count: values.length}]);
    store.lockedRows = null;
  }
}
function userPath(store, file, internal) {
  const absolute = path.resolve(String(file || ''));
  if (!file || typeof file !== 'string') fail('CONFIG.INVALID_REQUEST');
  const roots = internal ? [...store.s.roots, path.dirname(absolute)] : store.s.roots;
  const resolved = jail(absolute, roots);
  if (store.s.protectedRoots.some(root => inside(resolved, root))) fail('CONFIG.PATH_OUTSIDE_ROOT', 'Cannot use an installed plugin path');
  if (inside(resolved, store.s.repository)) fail('CONFIG.PATH_OUTSIDE_ROOT', 'Cannot use the case repository');
  return resolved;
}
export function projectFileSnapshot(store) {
  if (!store.s.projectFilesDir) return null;
  let items = listDocs(store);
  if (!items.length) {
    writeDoc(store, documentFromRepository(store, 'demo'));
    items = listDocs(store);
  }
  let active = readPointer(store);
  if (!items.some(item => item.id === active)) {
    active = (items.find(item => item.id === 'demo') || items[0]).id;
    writePointer(store, active);
  }
  return {active, items};
}
export function activateProjectFile(store, params) {
  return store.withLock(() => {
    if (store.runLocks()) fail('CONFIG.LOCKED_BY_RUN');
    projectFileSnapshot(store);
    persistActive(store);
    const doc = readDoc(store, params.id);
    materialize(store, doc);
    writePointer(store, doc.id);
    return {id: doc.id, name: doc.name, cases: doc.cases.length};
  });
}
export function importProjectFile(store, params, {internal = false} = {}) {
  return store.withLock(() => {
    if (store.runLocks()) fail('CONFIG.LOCKED_BY_RUN');
    const source = userPath(store, params.file, internal);
    const parsed = readJson(source, store.s.maxTotalBytes + 1048576);
    projectFileSnapshot(store);
    const taken = new Set(listDocs(store).map(item => item.id.toLowerCase()));
    let projectId = typeof parsed.id === 'string' ? parsed.id : path.basename(source, '.json');
    if (taken.has(String(projectId).toLowerCase())) {
      const stem = String(projectId).slice(0, 48);
      let n = 2;
      while (taken.has(`${stem}_${n}`.toLowerCase())) n += 1;
      projectId = `${stem}_${n}`;
    }
    const doc = normalize({...parsed, id: projectId}, projectId);
    checkedCases(store, doc.cases);
    if (store.projectDefinition && !doc.project) fail('CONFIG.IMPORT_REJECTED', 'Project settings are required');
    persistActive(store);
    writeDoc(store, doc);
    materialize(store, doc);
    writePointer(store, doc.id);
    return {id: doc.id, name: doc.name, cases: doc.cases.length};
  });
}
export function exportProjectFile(store, params, {internal = false} = {}) {
  return store.withLock(() => {
    const snap = projectFileSnapshot(store);
    persistActive(store);
    const doc = readDoc(store, snap.active);
    const file = userPath(store, params.file, internal);
    if (inside(file, catalogDir(store)) && path.basename(file) !== `${doc.id}.json`) fail('CONFIG.PATH_OUTSIDE_ROOT', 'Export cannot overwrite another project');
    atomicWrite(file, JSON.stringify(doc, null, 2) + '\n');
    return {id: doc.id, cases: doc.cases.length, file};
  });
}
