import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import {loadSchema,loadUi,redactSchema} from '../lib/hub/schema.mjs';
import {inspectFile} from '../lib/hub/file-inspect.mjs';
import crypto from 'node:crypto';

test('explicit file inspection hashes actual bytes and returns a portable jailed reference',t=>{
 const root=fs.mkdtempSync(path.join(os.tmpdir(),'hub-file-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
 const repository=path.join(root,'cases');fs.mkdirSync(repository);const file=path.join(root,'firmware.bin');fs.writeFileSync(file,'actual bytes');
 const store={s:{repository,dataRoot:root,roots:[root]},ui:{widgets:{'firmware.path':{type:'file',hash_field:'firmware.sha256'}}},schema:{schema:{properties:{firmware:{properties:{path:{type:'string',format:'path','x-path-base':'config_dir'},sha256:{type:'string'}}}}},documents:{}}};
 const data=inspectFile(store,{config_id:'case',field:'firmware.path',path:file,expected_value:'old.bin'});
 assert.equal(data.path,'../firmware.bin');assert.equal(data.sha256,crypto.createHash('sha256').update('actual bytes').digest('hex'));assert.equal(data.bytes,12);assert.equal(data.expected_value,'old.bin');
 assert.throws(()=>inspectFile(store,{field:'firmware.path',path:path.resolve(root,'../outside.bin')}),/PATH_OUTSIDE_ROOT/);
 assert.throws(()=>inspectFile(store,{field:'firmware.sha256',path:file}),/INVALID_REQUEST/);
});
test('Draft 2020-12 conditional, local references, formats and strict unknown fields',t=>{
 const root=fs.mkdtempSync(path.join(os.tmpdir(),'hub-schema-'));t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
 fs.writeFileSync(path.join(root,'number.json'),JSON.stringify({type:'number',minimum:1,maximum:9,multipleOf:0.5}));
 const schema={type:'object',additionalProperties:false,required:['mode','value','tags','when'],properties:{mode:{enum:['a','b']},value:{$ref:'number.json'},tags:{type:'array',maxItems:3,uniqueItems:true,items:{type:'string',minLength:1,maxLength:8}},when:{type:'string',format:'date-time'},extra:{type:'string'},opaque:{type:'string',writeOnly:true}},if:{properties:{mode:{const:'a'}}},then:{required:['extra']},dependentRequired:{opaque:['extra']}};
 const file=path.join(root,'config.json');fs.writeFileSync(file,JSON.stringify(schema));const loaded=loadSchema(file,root);
 const valid={mode:'a',value:1.5,tags:['ok'],when:new Date().toISOString(),extra:'present',opaque:'private'};assert.deepEqual(loaded.errors(valid),[]);
 for(const candidate of [{...valid,typo:1},{...valid,value:0},{...valid,tags:['x','x']},{...valid,when:'bad'},{...valid,extra:undefined}])assert.ok(loaded.errors(candidate).length);
 assert.equal(redactSchema(valid,loaded.schema,loaded.documents).opaque,'[REDACTED]');
 fs.writeFileSync(file,JSON.stringify({$ref:'https://example.invalid/schema'}));assert.throws(()=>loadSchema(file,root),/PATH_OUTSIDE_ROOT/);
 fs.writeFileSync(file,JSON.stringify({type:'object',patternProperties:{'.*':{type:'string'}}}));assert.throws(()=>loadSchema(file,root),/UNSUPPORTED_KEYWORD/);
 const ui=path.join(root,'ui.json');fs.writeFileSync(ui,JSON.stringify({description:'Help text',layout:{groups:[]},widgets:{unknown:{type:'unknown_widget'}}}));assert.equal(loadUi(ui,root).description,'Help text');
 fs.writeFileSync(ui,JSON.stringify({onClick:'execute()'}));assert.throws(()=>loadUi(ui,root),/UI_SCHEMA_INVALID/);
 fs.writeFileSync(ui,JSON.stringify({required_widgets:['not_implemented']}));assert.throws(()=>loadUi(ui,root),/UI_WIDGET_UNSUPPORTED/);
 fs.writeFileSync(ui,JSON.stringify({required_widgets:['rule_builder'],fields:{'rules.*.minimum':{order:1,width:'short',advanced:false}}}));assert.equal(loadUi(ui,root).fields['rules.*.minimum'].width,'short');
 fs.writeFileSync(ui,JSON.stringify({fields:{name:{width:'huge'}}}));assert.throws(()=>loadUi(ui,root),/UI_SCHEMA_INVALID/);
});
