import test from 'node:test';
import assert from 'node:assert/strict';
import {engineeringPrompt} from '../src/engineeringPrompt.ts';

test('engineering request includes only explicit context and the last eight selected turns',()=>{
  assert.equal(engineeringPrompt('  Check this result  ','',[]),'User question:\nCheck this result');
  const turns=Array.from({length:12},(_,index)=>({role:'user',text:`marker-${index}-end`}));
  const text=engineeringPrompt('question','VIN=12',turns);
  assert(!text.includes('marker-3-end'));
  assert(text.includes('marker-4-end'));
  assert(text.includes('marker-11-end'));
  assert(text.includes('User-provided context:\nVIN=12'));
  assert(!engineeringPrompt('question','',[]).includes('marker-'));
});
test('engineering request uses the bridge UTF-8 byte budget and rejects invalid input',()=>{
  assert.throws(()=>engineeringPrompt('串'.repeat(22000),'',[]),/64 KiB/);
  assert.throws(()=>engineeringPrompt('','',[]));
  assert.throws(()=>engineeringPrompt('a\0b','',[]));
  assert(engineeringPrompt('串'.repeat(20000),'',[]).length>20000);
});
