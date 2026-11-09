import test from 'node:test';
import assert from 'node:assert/strict';
import {resolveSchema,schemaDefault,variantIndex} from '../src/schemaForm.ts';

test('referenced configuration forms preserve required fields and choose the actual rule variant',()=>{
  const documents={rules:{$defs:{range:{type:'object',required:['type','minimum'],properties:{type:{const:'range'},minimum:{type:'number'},maximum:{type:'number'}}}}}};
  const schema={$ref:'rules#/$defs/range'};
  assert.equal(resolveSchema(schema,documents,{}).properties.type.const,'range');
  assert.deepEqual(schemaDefault(schema,documents,{}),{type:'range',minimum:0});
  assert.equal(variantIndex([{properties:{type:{const:'contains'}}},schema],{type:'range'},documents,{}),1);
});
test('readOnly constants and defaults keep their actual JSON types',()=>{
  const schema={type:'object',required:['enabled','count','name'],properties:{enabled:{type:'boolean',default:true},count:{type:'integer',minimum:1},name:{type:'string'},optional:{type:'string'}}};
  assert.deepEqual(schemaDefault(schema,{},schema),{enabled:true,count:1,name:''});
  assert.deepEqual(schemaDefault({type:'array'},{},{}),[]);
});

test('nullable and primitive schema alternatives retain their actual variant',()=>{
  const branches=[{type:'null'},{type:'number'},{type:'string'}];
  assert.equal(variantIndex(branches,null,{},{}),0);
  assert.equal(variantIndex(branches,12,{},{}),1);
  assert.equal(variantIndex(branches,'12',{},{}),2);
  assert.equal(schemaDefault({type:'null'},{},{}),null);
});
