import fs from 'node:fs';
import path from 'node:path';
import {fail, jail, id, readJson,sha,inside} from './common.mjs';

export function validateUi(manifest) {
  const configure = manifest.capabilities?.includes('configure');
  if (!configure) return null;
  const ui = manifest.ui, e = ui?.config_editor;
  if (!ui || ui.mode !== 'test_matrix') fail('PLUGIN.UI_SCHEMA_INVALID', 'configure requires ui.mode=test_matrix');
  if (!e || !e.schema || !e.repository || !ui.matrix_status_file) fail('PLUGIN.UI_SCHEMA_INVALID', 'Missing workspace paths');
  if (!Number.isInteger(ui.max_items)) fail('PLUGIN.UI_SCHEMA_INVALID','ui.max_items is required');
  for (const n of [ui.max_items,e.max_items]) if (n !== undefined && (!Number.isInteger(n) || n < 1 || n > 500)) fail('PLUGIN.UI_SCHEMA_INVALID','max_items must be 1–500');
  for (const [key,max] of [['max_item_bytes',1048576],['max_total_bytes',33554432]]) if (e[key] !== undefined && (!Number.isInteger(e[key]) || e[key] < 1 || e[key] > max)) fail('PLUGIN.UI_SCHEMA_INVALID',`Invalid ${key}`);
  if (e.source_view && !['hidden','read_only'].includes(e.source_view)) fail('PLUGIN.UI_SCHEMA_INVALID','Source must be read-only');
  if(ui.project_config && (ui.project_config.protocol!=='wiparse.project-config/1'||!ui.project_config.schema))fail('PLUGIN.UI_SCHEMA_INVALID','Invalid project configuration declaration');
  for (const f of [e.schema,e.ui_schema,ui.project_config?.schema,ui.project_config?.ui_schema].filter(Boolean)) if (path.isAbsolute(f) || /^[A-Za-z]:/.test(f) || f.split(/[\\/]/).includes('..')) fail('PLUGIN.UI_SCHEMA_INVALID','Schema path must be plugin-relative');
  return ui;
}
export function workspace(manifest, context) {
  const ui = validateUi(manifest); if (!ui) fail('PLUGIN.UI_SCHEMA_INVALID','Plugin does not declare configure');
  const {pluginDir, dataRoot, project = 'default', testId = manifest.report_name || manifest.id, params = {}, approvedRoots = []} = context;
  id(project); id(manifest.id); id(testId);
  const root = path.resolve(dataRoot), pluginRoot = fs.realpathSync.native(pluginDir);
  const testDir = path.join(root,'projects',project,'tests',testId);
  const vars = {data_root:root,project,plugin_id:manifest.id,test_dir:testDir};
  for (const p of manifest.params || []) if (p.type === 'path') vars[p.name] = params[p.name] ?? p.default;
  function expand(template, allowTest = false) {
    return String(template).replace(/\{(\w+)\}/g, (_,key) => {
      if ((!allowTest && key === 'test_dir') || !Object.hasOwn(vars,key) || !vars[key] || /\{\w+\}/.test(String(vars[key]))) fail('PLUGIN.UI_SCHEMA_INVALID',`Unresolved template ${key}`);
      return String(vars[key]);
    });
  }
  const e = ui.config_editor;
  const protectedRoots=[pluginRoot,...(context.protectedRoots||[])].filter(Boolean).map(p=>path.resolve(p));
  const repository = jail(path.resolve(root,expand(e.repository)),[root,...approvedRoots]);
  if (protectedRoots.some(p=>inside(repository,p))) fail('CONFIG.PATH_OUTSIDE_ROOT','Repository cannot be inside plugin installation');
  // Host-owned metadata stays under data_root, even when an explicitly approved
  // repository is external. Project-config plugins share one scope lock/journal
  // across repositories so shared settings cannot bypass CAS or active-run locks.
  const editorRoot = jail(path.join(root,'projects',project,'.editor',testId,manifest.id,ui.project_config?'project-scope':sha(repository.toLowerCase()).slice(0,16)),[root]);
  const archive = jail(e.archive_dir ? path.resolve(root,expand(e.archive_dir)) : path.join(repository,'_trash'),[repository]);
  const controlFile=ui.execution_controls?.control_file?jail(path.resolve(root,expand(ui.execution_controls.control_file,true)),[testDir]):null;
  const audit = jail(e.audit_file ? path.resolve(root,expand(e.audit_file)) : path.join(editorRoot,'config-audit.jsonl'),[root,...approvedRoots]);
  const normalizedParams={};
  for(const p of manifest.params||[]){
    if(p.name==='preflight_only')continue;let v=params[p.name]??p.default;if(v===undefined)continue;
    if(p.type==='number'||p.type==='device'){const n=Number(v);if(v!==''&&v!==null&&Number.isFinite(n))v=n;}
    else if(p.type==='boolean')v=v===true||/^(1|true|yes|on)$/i.test(String(v));
    else if(p.type==='json'){if(typeof v==='string'&&v.trim()){try{v=JSON.parse(v);}catch{}}}
    else if(v!==null)v=String(v);
    normalizedParams[p.name]=v;
  }
  return {plugin:manifest, params:normalizedParams, pluginDir:pluginRoot, dataRoot:root, project, testDir,controlFile,
    repository, editorRoot, archive, audit, roots:[root,...approvedRoots],protectedRoots,
    projectFile:jail(path.join(testDir,'.hub',manifest.id,'project-config.json'),[testDir]),
    matrix: jail(path.resolve(root,expand(ui.matrix_status_file,true)),[testDir]),
    schema: jail(path.join(pluginRoot,e.schema),[pluginRoot]),
    uiSchema: e.ui_schema ? jail(path.join(pluginRoot,e.ui_schema),[pluginRoot]) : null,
    maxItems: Math.min(ui.max_items || 500,e.max_items || 500), maxItemBytes:e.max_item_bytes || 1048576, maxTotalBytes:e.max_total_bytes || 33554432};
}

