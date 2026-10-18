import {projectDefinition,getProject,projectSnapshot} from './project-config.mjs';
import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {spawnSync} from 'node:child_process';
import {fileURLToPath} from 'node:url';
import {atomicWrite,appendSync,flushDir,jail,id,sha,canonical,readJson,fail,page,redact,inside} from './common.mjs';
import {loadSchema,loadUi,defaults} from './schema.mjs';

const alive = pid => { try { process.kill(pid,0); return true; } catch (e) { return e.code !== 'ESRCH'; } };
export class ConfigStore {
  constructor(space, {fault = null} = {}) {
    this.s=space; this.fault=fault;
    for (const dir of [space.repository,space.editorRoot,space.archive]) fs.mkdirSync(jail(dir,space.roots),{recursive:true});
    this.transactions=path.join(space.editorRoot,'transactions'); fs.mkdirSync(this.transactions,{recursive:true});
    this.schema=loadSchema(space.schema,space.pluginDir); this.ui=loadUi(space.uiSchema,space.pluginDir);
    this.projectDefinition=projectDefinition(this);
    this.schemaHash=sha(canonical({schema:this.schema.fingerprint,ui:this.ui,project_schema:this.projectDefinition?.schema.fingerprint??null,project_ui:this.projectDefinition?.ui??null,plugin:readJson(path.join(space.pluginDir,'plugin.json')),entry:sha(fs.readFileSync(jail(path.join(space.pluginDir,space.plugin.entry||'index.mjs'),[space.pluginDir])))}));
  }
  withLock(action) {
    const mutex=path.join(this.s.editorRoot,'write.lock');
    try { fs.mkdirSync(mutex); }
    catch (e) {
      if (e.code !== 'EEXIST') throw e;
      // A live owner's ordinary read/write is not a recovery attempt. Avoid a
      // competing recovery gate turning short read contention into a false
      // RECOVERY_REQUIRED, and identify acquisition-only retries explicitly.
      let preliminary;try{preliminary=readJson(path.join(mutex,'owner.json'));}catch{fail('CONFIG.BUSY','Repository lock initialization is in progress',{lock_contention:true});}
      if(Number.isInteger(preliminary.pid)&&preliminary.pid>0&&alive(preliminary.pid))fail('CONFIG.BUSY','Another workspace operation is in progress',{lock_contention:true});
      const gate=path.join(this.s.editorRoot,'recovery.lock');
      try{fs.mkdirSync(gate);}catch{fail('CONFIG.RECOVERY_REQUIRED','Recovery is already active or was interrupted; repository remains read-only');}
      try{
        let owner; try { owner=readJson(path.join(mutex,'owner.json')); } catch { fail('CONFIG.BUSY','Repository lock initialization is in progress',{lock_contention:true}); }
        if (!Number.isInteger(owner.pid)||owner.pid<=0)fail('CONFIG.RECOVERY_REQUIRED','Invalid repository lock owner');
        if (alive(owner.pid)) fail('CONFIG.BUSY','Another workspace operation is in progress',{lock_contention:true});
        fs.rmSync(mutex,{recursive:true}); fs.mkdirSync(mutex);
        atomicWrite(path.join(mutex,'owner.json'),JSON.stringify({pid:process.pid,nonce:crypto.randomUUID()}));
        appendSync(this.s.audit,JSON.stringify({at:new Date().toISOString(),operation:'recover_stale_lock',actor:'local-user',previous_pid:owner.pid})+'\n');
      }finally{fs.rmdirSync(gate);}
    }
    atomicWrite(path.join(mutex,'owner.json'),JSON.stringify({pid:process.pid,nonce:crypto.randomUUID()}));
    this.inLock=true;this.lockedRows=null;
    try { this.recover(); return action(); }
    finally { this.inLock=false;this.lockedRows=null;fs.rmSync(mutex,{recursive:true,force:true}); }
  }
  file(configId) {
    const file=path.join(this.s.repository,id(configId)+'.json');
    try{if(fs.lstatSync(file).isSymbolicLink())fail('CONFIG.PATH_OUTSIDE_ROOT','Configuration links are not allowed');}catch(e){if(e.code!=='ENOENT')throw e;}
    return file;
  }
  scan() {
    if(this.inLock&&this.lockedRows)return this.lockedRows;
    // Revalidate the directory chain once per scan, then each direct child.
    // Identifiers cannot contain separators; avoid 500 repeated parent traversals.
    jail(this.s.repository,this.s.roots);
    const rows=[]; let total=0;
    for (const name of fs.readdirSync(this.s.repository)) {
      if (!name.endsWith('.json')) continue;
      if (rows.length >= this.s.maxItems) fail('CONFIG.REPOSITORY_LIMIT','Too many configurations');
      const file=this.file(name.slice(0,-5)),stat=fs.statSync(file);
      total+=stat.size;if(!stat.isFile()||stat.size>this.s.maxItemBytes||total>this.s.maxTotalBytes)fail('CONFIG.REPOSITORY_LIMIT','Repository size limit exceeded');
      const bytes=fs.readFileSync(file);
      let value; try { value=JSON.parse(bytes); } catch { value={id:name.slice(0,-5),name:name,status:'invalid'}; }
      if(!value||typeof value!=='object'||Array.isArray(value))value={id:name.slice(0,-5),name,status:'invalid'};
      const errors=this.schema.errors(value); if (value.id !== name.slice(0,-5)) errors.push({path:'/id',message:'Filename and id differ'});
      rows.push({id:name.slice(0,-5),name:String(value.name || name).slice(0,128),tags:Array.isArray(value.tags)?value.tags.filter(t=>typeof t==='string').slice(0,32).map(t=>t.slice(0,64)):[],enabled:value.enabled !== false,order:Number.isFinite(value.order)?value.order:0,updated_at:typeof value.updated_at==='string'?value.updated_at.slice(0,40):'',revision:Number.isSafeInteger(value.editor_revision)&&value.editor_revision>=0?value.editor_revision:0,sha256:sha(bytes),status:errors.length ? 'invalid' : 'valid',errors,value,bytes});
    }
    if (new Set(rows.map(r=>r.id.toLowerCase())).size !== rows.length) fail('CONFIG.ID_CONFLICT','Case-insensitive id collision');
    if(this.inLock)this.lockedRows=rows;
    return rows;
  }
  get(configId) { const row=this.scan().find(r=>r.id===configId); if (!row) fail('CONFIG.NOT_FOUND'); return row; }
  list(params={}) {
    const rows=this.scan().map(({value,bytes,...row})=>row);
    return {...page(rows,params),schema_fingerprint:this.schemaHash};
  }
  draft(configId, value) {
    id(configId); if (this.scan().some(r=>r.id.toLowerCase()===configId.toLowerCase())) fail('CONFIG.ID_CONFLICT');
    value=structuredClone(value || defaults(this.schema.schema,this.schema.documents));
    Object.assign(value,{id:configId,editor_revision:0,updated_at:new Date().toISOString()});
    if (!value.name) value.name=configId;
    return {config_id:configId,value,revision:0,sha256:'',errors:this.schema.errors(value),draft:true};
  }
  checkPathFields(value,schema=this.schema.schema) {
    if (!schema || !value) return;
    if (schema.$ref) {
      const [key,fragment]=schema.$ref.split('#'); let target=this.schema.documents[key];
      if (fragment) for (const p of fragment.split('/').slice(1)) target=target?.[p.replaceAll('~1','/').replaceAll('~0','~')];
      return this.checkPathFields(value,target);
    }
    if (typeof value === 'string' && ['path','file-path','directory'].includes(schema.format) && value) jail(path.resolve(schema['x-path-base']==='config_dir'?this.s.repository:this.s.dataRoot,value),this.s.roots);
    if (Array.isArray(value)) for (const v of value) this.checkPathFields(v,schema.items);
    else if (typeof value === 'object') for (const [k,v] of Object.entries(value)) this.checkPathFields(v,schema.properties?.[k]);
    for(const key of ['allOf','oneOf','anyOf'])for(const branch of schema[key]||[])this.checkPathFields(value,branch);
    for(const key of ['then','else'])if(schema[key])this.checkPathFields(value,schema[key]);
  }
  validate(values,projectConfig=undefined) {
    for (const value of values) {
      const errors=this.schema.errors(value); if (errors.length) fail('SCHEMA.VALIDATION_FAILED','Configuration is invalid',{errors});
      this.checkPathFields(value);
    }
    if (!values.length) return [];
    if(projectConfig===undefined&&this.projectDefinition){const p=getProject(this);projectConfig=p.saved?projectSnapshot(this):null;}
    const script=fileURLToPath(new URL('./configure-worker.mjs',import.meta.url));
    const child=spawnSync(process.execPath,[script],{input:JSON.stringify({pluginDir:this.s.pluginDir,entry:this.s.plugin.entry || 'index.mjs',values,projectConfig}),encoding:'utf8',timeout:10000,maxBuffer:4194304,windowsHide:true});
    let result; try { result=JSON.parse(child.stdout); } catch { fail('SCHEMA.VALIDATION_FAILED','Plugin configure crashed, timed out or returned invalid data'); }
    if (child.error || child.status !== 0 || result.ok !== true) fail('SCHEMA.VALIDATION_FAILED','Plugin configure rejected configuration',{errors:result.errors || []});
    this.normalized=result.normalized_values||[];
    return result.warnings || [];
  }
  runLocks() {
    const lock=path.join(this.s.editorRoot,'run-binding.json'); if (!fs.existsSync(lock)) return null;
    const binding=readJson(jail(lock,this.s.roots));
    if (!alive(binding.pid)) { fs.unlinkSync(lock); appendSync(this.s.audit,JSON.stringify({at:new Date().toISOString(),operation:'recover_run_lock',actor:'local-user',reason:'owner_process_exited',run_id:binding.run_id})+'\n'); return null; }
    return binding;
  }
  assertUnlocked(ids) {
    const binding=this.runLocks();
    if (binding && (binding.schema_hash !== this.schemaHash || ids.some(id=>binding.ids.includes(id)))) fail('CONFIG.LOCKED_BY_RUN','Active run freezes these configurations');
  }
  expected(row,params) {
    if ((row?.revision || 0) !== params.expected_revision || (row?.sha256 || '') !== params.expected_sha256) fail('CONFIG.REVISION_CONFLICT','Reload before saving',{current_revision:row?.revision || 0,current_sha256:row?.sha256 || ''});
  }
  commit(entries,events) {
    const txId=crypto.randomUUID(); const intent=path.join(this.transactions,txId+'.json');
    const auditBefore=fs.existsSync(this.s.audit) ? fs.statSync(this.s.audit).size : 0;
    const changes=entries.map(({file,bytes})=>({file:jail(file,this.s.roots),old:fs.existsSync(file)?fs.readFileSync(file).toString('base64'):null,next:bytes===null?null:Buffer.from(bytes).toString('base64')}));
    const audit=events.map(event=>JSON.stringify({at:new Date().toISOString(),actor:'local-user',plugin_id:this.s.plugin.id,tx_id:txId,...event})+'\n').join('');
    const journal={tx_id:txId,repository:this.s.repository,auditFile:this.s.audit,changes,auditBefore,audit}; atomicWrite(intent,JSON.stringify(journal));
    this.fault?.('before_replace');
    try {
      for (const change of changes) this.apply(change.file,change.next,journal.repository);
      this.fault?.('after_replace');
      appendSync(this.s.audit,audit);
      this.fault?.('after_audit');
      atomicWrite(intent+'.committed','committed');
      fs.unlinkSync(intent); fs.unlinkSync(intent+'.committed'); flushDir(this.transactions);
    } catch (error) {
      try { this.rollback(journal); fs.unlinkSync(intent); }
      catch { atomicWrite(path.join(this.s.editorRoot,'recovery-required'),'true'); fail('CONFIG.RECOVERY_REQUIRED','Transaction rollback failed; repository is read-only'); }
      throw error;
    }
    const preflight=path.join(this.s.editorRoot,'preflight.json'); if (fs.existsSync(preflight)) fs.unlinkSync(preflight);
  }
  apply(file,base64,origin=this.s.repository) { origin=jail(origin,this.s.roots);if(this.s.protectedRoots.some(root=>inside(origin,root)))fail('CONFIG.PATH_OUTSIDE_ROOT'); file=jail(file,[origin,path.dirname(this.s.projectFile)]); if(this.s.protectedRoots.some(root=>inside(file,root)))fail('CONFIG.PATH_OUTSIDE_ROOT'); if (base64===null) { if (fs.existsSync(file)) fs.unlinkSync(file); flushDir(path.dirname(file)); } else atomicWrite(file,Buffer.from(base64,'base64')); }
  rollback(journal) {
    for (const change of journal.changes) this.apply(change.file,change.old,journal.repository);
    const auditFile=jail(journal.auditFile||this.s.audit,this.s.roots);
    if (fs.existsSync(auditFile)) { const fd=fs.openSync(auditFile,'r+'); try { fs.ftruncateSync(fd,journal.auditBefore); fs.fsyncSync(fd); } finally { fs.closeSync(fd); } }
  }
  recover() {
    for (const name of fs.readdirSync(this.transactions).filter(n=>n.endsWith('.json'))) {
      const file=jail(path.join(this.transactions,name),this.s.roots), journal=readJson(file,256*1048576);
      const auditFile=jail(journal.auditFile||this.s.audit,this.s.roots);const audit=fs.existsSync(auditFile)?fs.readFileSync(auditFile):Buffer.alloc(0);
      const committed=audit.subarray(journal.auditBefore,journal.auditBefore+Buffer.byteLength(journal.audit)).toString()===journal.audit && journal.changes.every(c=>c.next===null?!fs.existsSync(jail(c.file,this.s.roots)):fs.existsSync(jail(c.file,this.s.roots))&&sha(fs.readFileSync(c.file))===sha(Buffer.from(c.next,'base64')));
      if (!committed) this.rollback(journal);
      fs.unlinkSync(file); if (fs.existsSync(file+'.committed')) fs.unlinkSync(file+'.committed');
    }
    const recovery=path.join(this.s.editorRoot,'recovery-required'); if (fs.existsSync(recovery)) fs.unlinkSync(recovery);
    for (const dir of [this.s.repository,this.transactions]) for (const file of fs.readdirSync(dir).filter(n=>n.startsWith('.')&&n.endsWith('.tmp'))) fs.unlinkSync(jail(path.join(dir,file),this.s.roots));
  }
  save(params, operation='save') {
    return this.withLock(()=>{
      const rows=this.scan(), old=rows.find(r=>r.id===params.config_id);
      this.expected(old,params); this.assertUnlocked([params.config_id]);
      const value=structuredClone(params.value);if(!value||typeof value!=='object'||Array.isArray(value))fail('SCHEMA.VALIDATION_FAILED','Configuration must be an object');id(value.id);this.assertUnlocked([params.config_id,value.id]);
      if(old&&old.id!==value.id&&old.id.toLowerCase()===value.id.toLowerCase())fail('CONFIG.ID_CONFLICT','Case-only rename is ambiguous on Windows; choose a distinct id');
      if (rows.some(r=>r.id.toLowerCase()===value.id.toLowerCase()&&r.id!==old?.id)) fail('CONFIG.ID_CONFLICT');
      Object.assign(value,{editor_revision:(old?.revision || 0)+1,updated_at:new Date().toISOString()});
      const warnings=this.validate([value]); const bytes=Buffer.from(JSON.stringify(value,null,2)+'\n');
      if (bytes.length>this.s.maxItemBytes || rows.length+(old?0:1)>this.s.maxItems || rows.reduce((n,r)=>n+r.bytes.length,0)-(old?.bytes.length || 0)+bytes.length>this.s.maxTotalBytes) fail('CONFIG.REPOSITORY_LIMIT');
      if (params.validate_only) return {valid:true,errors:[],warnings,normalized_value:this.normalized?.[0]??null};
      const entries=[{file:this.file(value.id),bytes}]; if (old && old.id!==value.id) entries.push({file:this.file(old.id),bytes:null});
      this.commit(entries,[{config_id:value.id,operation:old?(old.id!==value.id?'rename':operation):(operation==='save'?'create':operation),old_revision:old?.revision || 0,new_revision:value.editor_revision,old_sha256:old?.sha256 || '',new_sha256:sha(bytes)}]);
      return {config_id:value.id,revision:value.editor_revision,sha256:sha(bytes),valid:true,warnings,preflight_invalidated:true,value};
    });
  }
  archive(params) {
    return this.withLock(()=>{
      const old=this.get(params.config_id); this.expected(old,params); this.assertUnlocked([old.id]);
      const name=`${old.id}.${crypto.randomUUID()}.json`;
      this.commit([{file:path.join(this.s.archive,name),bytes:JSON.stringify({value:old.value,original_id:old.id,sha256:old.sha256,archived_at:new Date().toISOString(),actor:'local-user'})},{file:this.file(old.id),bytes:null}],[{config_id:old.id,operation:'archive',old_revision:old.revision,old_sha256:old.sha256,new_sha256:null}]);
      return {archived:true,archive_id:name,preflight_invalidated:true};
    });
  }
  archives() { return fs.readdirSync(this.s.archive).filter(n=>/^[A-Za-z0-9._-]+\.json$/.test(n)).slice(0,500).map(name=>{const item=readJson(jail(path.join(this.s.archive,name),[this.s.archive]),this.s.maxItemBytes+4096); return {archive_id:name,id:item.original_id,name:item.value?.name,archived_at:item.archived_at,sha256:item.sha256};}); }
  restore(params) {
    if (!/^[A-Za-z0-9._-]{1,180}\.json$/.test(params.archive_id || '')) fail('CONFIG.NOT_FOUND');
    const file=jail(path.join(this.s.archive,params.archive_id),[this.s.archive]); const item=readJson(file,this.s.maxItemBytes+4096);
    const draft=this.draft(params.config_id || item.original_id,item.value);
    const result=this.save({...params,config_id:draft.config_id,value:draft.value,expected_revision:0,expected_sha256:''},'restore');
    // Archive is retained as history; restore does not silently destroy evidence.
    return result;
  }
  importBundle(params) {
    return this.withLock(()=>{
      const source=jail(path.resolve(this.s.dataRoot,params.file),this.s.roots);
      const bundle=readJson(source,this.s.maxTotalBytes+1048576);
      if (bundle.schema_version!=='wiparse.config-bundle/1' || bundle.plugin_id!==this.s.plugin.id || !Array.isArray(bundle.items) || bundle.items.length>this.s.maxItems) fail('CONFIG.IMPORT_REJECTED','Invalid bundle');
      const staging=path.join(this.s.editorRoot,'import-staging',crypto.randomUUID()+'.json'); atomicWrite(staging,JSON.stringify(bundle));
      try {
        const rows=this.scan(), byId=new Map(rows.map(r=>[r.id.toLowerCase(),r])), entries=[],events=[],values=[];
        for (let candidate of bundle.items) {
          candidate=structuredClone(candidate); id(candidate.id); let old=byId.get(candidate.id.toLowerCase());
          if (old) {
            if ((params.strategy || 'skip')==='skip') continue;
            if (params.strategy==='rename') { const stem=candidate.id.slice(0,48); let n=1; while(byId.has(`${stem}_${n}`.toLowerCase())) n++; candidate.id=`${stem}_${n}`; old=null; }
            else if (params.strategy==='replace') { this.expected(old,params.expected?.[old.id] || {}); this.assertUnlocked([old.id]);candidate.id=old.id; }
            else fail('CONFIG.IMPORT_REJECTED','Unknown conflict strategy');
          }
          if (values.some(v=>v.id.toLowerCase()===candidate.id.toLowerCase())) fail('CONFIG.ID_CONFLICT','Duplicate import id');
          Object.assign(candidate,{editor_revision:(old?.revision || 0)+1,updated_at:new Date().toISOString()});
          const bytes=Buffer.from(JSON.stringify(candidate,null,2)+'\n');
          if (bytes.length>this.s.maxItemBytes) fail('CONFIG.REPOSITORY_LIMIT');
          values.push(candidate); entries.push({file:this.file(candidate.id),bytes});
          events.push({operation:'import',config_id:candidate.id,old_revision:old?.revision || 0,new_revision:candidate.editor_revision,old_sha256:old?.sha256 || '',new_sha256:sha(bytes)});
          byId.set(candidate.id.toLowerCase(),{id:candidate.id,bytes});
        }
        if (byId.size>this.s.maxItems || [...byId.values()].reduce((n,r)=>n+r.bytes.length,0)>this.s.maxTotalBytes) fail('CONFIG.REPOSITORY_LIMIT');
        this.assertUnlocked(values.map(v=>v.id));
        const warnings=this.validate(values);
        if (entries.length) this.commit(entries,events);
        return {imported:entries.length,skipped:bundle.items.length-entries.length,warnings,preflight_invalidated:!!entries.length};
      } finally { fs.unlinkSync(staging); }
    });
  }
  exportBundle(params) {
    return this.withLock(()=>{
      if(params.ids!==undefined&&(!Array.isArray(params.ids)||params.ids.length>500))fail('CONFIG.INVALID_REQUEST');
      const rows=this.scan().filter(r=>params.ids===undefined || params.ids.includes(r.id));
      const file=jail(path.resolve(this.s.dataRoot,params.file),this.s.roots);
      if(this.s.protectedRoots.some(root=>inside(file,root)))fail('CONFIG.PATH_OUTSIDE_ROOT','Export cannot modify installed plugins');
      if (insideRepository(file,this.s)) fail('CONFIG.PATH_OUTSIDE_ROOT','Export cannot overwrite the repository');
      const bundle={schema_version:'wiparse.config-bundle/1',plugin_id:this.s.plugin.id,plugin_version:this.s.plugin.version,exported_at:new Date().toISOString(),items:rows.map(r=>r.value)};
      const old=fs.existsSync(file)?fs.readFileSync(file):null;
      atomicWrite(file,JSON.stringify(bundle,null,2)+'\n');
      try { appendSync(this.s.audit,JSON.stringify({at:new Date().toISOString(),actor:'local-user',plugin_id:this.s.plugin.id,operation:'export',item_count:rows.length})+'\n'); }
      catch (error) { if (old) atomicWrite(file,old); else fs.unlinkSync(file); throw error; }
      return {exported:rows.length,file:path.relative(this.s.dataRoot,file)};
    });
  }
  batch(params) {
    return this.withLock(()=>{
      if (!Array.isArray(params.ids) || params.ids.length>500) fail('CONFIG.REPOSITORY_LIMIT');
      if(typeof params.enabled!=='boolean'&&!Array.isArray(params.tags))fail('CONFIG.INVALID_REQUEST','Batch requires enabled or tags');
      if(new Set(params.ids).size!==params.ids.length) fail('CONFIG.ID_CONFLICT','Duplicate batch ids');
      this.assertUnlocked(params.ids); const all=this.scan(), rows=params.ids.map(key=>{const row=all.find(r=>r.id===key);if(!row) fail('CONFIG.NOT_FOUND');return row;}); const values=[];
      for (const row of rows) {
        this.expected(row,params.expected?.[row.id] || {}); const value=structuredClone(row.value);
        if (typeof params.enabled==='boolean') value.enabled=params.enabled;
        if (params.tags) value.tags=params.tags;
        value.editor_revision=row.revision+1; value.updated_at=new Date().toISOString(); values.push(value);
      }
      const warnings=this.validate(values);
      const entries=values.map(value=>({file:this.file(value.id),bytes:Buffer.from(JSON.stringify(value,null,2)+'\n')}));
      if (entries.some(e=>e.bytes.length>this.s.maxItemBytes)) fail('CONFIG.REPOSITORY_LIMIT');
      const size=this.scan().reduce((n,row)=>n+(params.ids.includes(row.id)?0:row.bytes.length),0)+entries.reduce((n,e)=>n+e.bytes.length,0); if(size>this.s.maxTotalBytes) fail('CONFIG.REPOSITORY_LIMIT');
      this.commit(entries,values.map((v,i)=>({operation:params.tags?'save':params.enabled?'enable':'disable',config_id:v.id,old_revision:rows[i].revision,new_revision:v.editor_revision,old_sha256:rows[i].sha256,new_sha256:sha(entries[i].bytes)})));
      return {updated:values.length,warnings,preflight_invalidated:true};
    });
  }
}
function insideRepository(file,s) { return [s.repository,s.editorRoot].some(root=>file===root || file.startsWith(root+path.sep)); }
