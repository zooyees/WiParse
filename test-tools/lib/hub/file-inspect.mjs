import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {jail,fail} from './common.mjs';

// Runs in the bounded Hub worker, never on the GUI thread. Inspection is an
// explicit user action; preflight/run still independently verify file content.
export function inspectFile(store, params) {
  const field=params.field;
  if(typeof field!=='string'||!/^[\w.-]+$/.test(field)||typeof params.path!=='string'||!params.path)fail('CONFIG.INVALID_REQUEST');
  function at(dot) {
    let schema=store.schema.schema;
    for(const key of dot.split('.')) {
      while(schema?.$ref){const [id,fragment]=schema.$ref.split('#');schema=store.schema.documents[id];if(fragment)for(const p of fragment.split('/').slice(1))schema=schema?.[p.replaceAll('~1','/').replaceAll('~0','~')];}
      schema=schema?.properties?.[key];
    }
    return schema;
  }
  const schema=at(field),widget=store.ui.widgets?.[field];
  if(!schema||!['path','file-path'].includes(schema.format))fail('CONFIG.INVALID_REQUEST','Not a file field');
  const base=schema['x-path-base']==='config_dir'?store.s.repository:store.s.dataRoot;
  const file=jail(path.resolve(base,params.path),store.s.roots);
  const before=fs.statSync(file,{bigint:true});
  if(!before.isFile()||before.size>1073741824n)fail('CONFIG.FILE_TOO_LARGE');
  const hash=crypto.createHash('sha256'),buffer=Buffer.alloc(65536),fd=fs.openSync(file,'r');
  try {for(;;){const n=fs.readSync(fd,buffer,0,buffer.length,null);if(!n)break;hash.update(buffer.subarray(0,n));}}finally{fs.closeSync(fd);}
  const after=fs.statSync(file,{bigint:true});
  if(before.ino!==after.ino||before.size!==after.size||before.mtimeNs!==after.mtimeNs||before.ctimeNs!==after.ctimeNs)fail('CONFIG.FILE_CHANGED');
  const hashField=typeof widget==='object'?widget.hash_field:null;
  if(hashField&&(!/^[\w.-]+$/.test(hashField)||at(hashField)?.type!=='string'))fail('PLUGIN.UI_SCHEMA_INVALID','Invalid hash target');
  return {config_id:params.config_id,field,pointer:'/'+field.replaceAll('.','/'),expected_value:params.expected_value,
    path:path.relative(base,file).replaceAll('\\','/'),sha256:hash.digest('hex'),bytes:Number(after.size),
    modified_ns:after.mtimeNs.toString(),hash_field:hashField??null};
}
