import test from 'node:test';
import assert from 'node:assert/strict';
import {scopeRuleItems} from '../src/scopeRuleDocument.ts';
test('scope triggers preserve array and items document contracts',()=>{
  assert.deepEqual(scopeRuleItems(''),[]);
  const item={id:'sample',type:'regex',pattern:'ASK',unless:[{type:'contains',text:'noise'}]};
  assert.deepEqual(scopeRuleItems(JSON.stringify([item])),[item]);
  assert.deepEqual(scopeRuleItems(JSON.stringify({version:2,items:[item],extra:'retained'})),[item]);
  assert.equal(scopeRuleItems('{broken'),null);
  assert.equal(scopeRuleItems('{}'),null);
});
