import test from 'node:test';
import assert from 'node:assert/strict';
import {bindVisibleOrigins,isPreservedValue} from '../src/visibleDocument.ts';
test('masked values retain original locations through edits and reordering',()=>{
  const original={rows:[{name:'a',file:'[LOCAL_PATH]'},{name:'b',key:'[REDACTED]'}]};
  const bound=bindVisibleOrigins(original);
  assert.equal(original.rows[0].file,'[LOCAL_PATH]');
  bound.rows.reverse();
  assert.equal(bound.rows[0].key,`[REDACTED@${encodeURIComponent(JSON.stringify(['rows','1','key']))}]`);
  assert.equal(isPreservedValue(bound.rows[0].key),true);
  assert.equal(isPreservedValue('capture.isf'),false);
});
