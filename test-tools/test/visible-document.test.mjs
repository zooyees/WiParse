import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {execute} from '../hub-workspace.mjs';
import test from 'node:test';
import assert from 'node:assert/strict';
import {restoreVisibleDocument} from '../lib/hub/visible-document.mjs';

test('full form edits preserve redacted file references by item identity when reordered',()=>{
  const stored={firmware:{path:'firmware/main.bin'},captures:[{id:'a',path:'a.isf'},{id:'b',path:'b.isf'}]};
  const visible={firmware:{path:'[LOCAL_PATH]'},captures:[{id:'b',path:'[LOCAL_PATH]'},{id:'a',path:'[LOCAL_PATH]'}],name:'edited'};
  const result=restoreVisibleDocument(visible,stored);
  assert.deepEqual(result.captures,[{id:'b',path:'b.isf'},{id:'a',path:'a.isf'}]);assert.equal(result.firmware.path,'firmware/main.bin');assert.equal(stored.name,undefined);
});
test('new placeholders and reserved object properties cannot become actual configuration',()=>{
  assert.throws(()=>restoreVisibleDocument({new:'[LOCAL_PATH]'},{}),error=>error.code==='CONFIG.REDACTED_VALUE');
  assert.throws(()=>restoreVisibleDocument(JSON.parse('{"__proto__":{"polluted":true}}'),{}),error=>error.code==='CONFIG.INVALID_REQUEST');
  assert.equal({}.polluted,undefined);
});

test('complete configuration edits retain revision and schema fences and validation does not write',t=>{
  const root=fs.mkdtempSync(path.join(os.tmpdir(),'wiparse-visible-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  const repo=relative=>fileURLToPath(new URL('../../'+relative,import.meta.url));
  const cases=path.join(root,'cases');fs.cpSync(repo('recipes/wireless-whitebox/cases'),cases,{recursive:true});
  const context={pluginDir:repo('test-tools/plugins/wireless-whitebox'),dataRoot:root,project:'default',testId:'WirelessWhitebox',params:{config_dir:cases},activePlugin:'wireless-whitebox'};
  const call=(method,params)=>execute({context,method,params,internal:false});
  const before=call('ui.test_tool.config.get',{config_id:'wcv_07'});
  const params={config_id:'wcv_07',expected_project:'default',expected_revision:before.revision,expected_sha256:before.sha256,schema_fingerprint:before.schema_fingerprint,value:{...before.value,name:'Complete form edit'}};
  assert.equal(call('ui.test_tool.config.edit',{...params,validate_only:true}).valid,true);
  assert.equal(call('ui.test_tool.config.get',{config_id:'wcv_07'}).sha256,before.sha256);
  const saved=call('ui.test_tool.config.edit',params);assert.equal(saved.value.name,'Complete form edit');assert.equal(saved.revision,before.revision+1);
  assert.throws(()=>call('ui.test_tool.config.edit',params),error=>error.code==='CONFIG.REVISION_CONFLICT');
  assert.throws(()=>call('ui.test_tool.config.edit',{...params,schema_fingerprint:'wrong'}),error=>error.code==='CONFIG.CONTEXT_CHANGED');
});

test('redacted structured credentials are preserved without returning them to the editor',()=>{
  const original={protected:{kind:'private',fields:[1,2]}};
  const result=restoreVisibleDocument({protected:'[REDACTED]'},original);
  assert.deepEqual(result,original);assert.notEqual(result.protected,original.protected);
});

test('preserved field references follow moved rows without exposing or swapping stored values',()=>{
  const stored={rules:[{name:'A',path:'capture-a.isf',token:'a'},{name:'B',path:'capture-b.isf',token:'b'}]};
  const ref=(kind,...path)=>`[${kind}@${encodeURIComponent(JSON.stringify(path))}]`;
  const visible={rules:[{name:'B edited',path:ref('LOCAL_PATH','rules','1','path'),token:ref('REDACTED','rules','1','token')},{name:'A',path:ref('LOCAL_PATH','rules','0','path'),token:ref('REDACTED','rules','0','token')}]};
  assert.deepEqual(restoreVisibleDocument(visible,stored),{rules:[{name:'B edited',path:'capture-b.isf',token:'b'},{name:'A',path:'capture-a.isf',token:'a'}]});
  assert.throws(()=>restoreVisibleDocument({x:ref('REDACTED','__proto__')},stored),error=>error.code==='CONFIG.REDACTED_VALUE');
  assert.throws(()=>restoreVisibleDocument({x:ref('REDACTED','rules','9','token')},stored),error=>error.code==='CONFIG.REDACTED_VALUE');
});
