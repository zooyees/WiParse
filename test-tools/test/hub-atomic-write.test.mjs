import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import {atomicWrite} from '../lib/hub/common.mjs';

test('Windows transient rename conflicts retry without deleting the destination', {skip:process.platform!=='win32'}, t=>{
  const root=fs.mkdtempSync(path.join(process.cwd(),'hub-atomic-test-'));
  t.after(()=>fs.rmSync(root,{recursive:true,force:true}));
  const file=path.join(root,'case.json');fs.writeFileSync(file,'original');
  const original=fs.renameSync;let attempts=0;
  try{
    fs.renameSync=(from,to)=>{assert.equal(fs.readFileSync(file,'utf8'),'original');if(attempts++<2)throw Object.assign(new Error('simulated sharing violation'),{code:'EPERM'});return original(from,to);};
    atomicWrite(file,'updated');
  }finally{fs.renameSync=original;}
  assert.equal(attempts,3);assert.equal(fs.readFileSync(file,'utf8'),'updated');assert.deepEqual(fs.readdirSync(root),['case.json']);
  attempts=0;
  try{
    fs.renameSync=()=>{attempts++;throw Object.assign(new Error('persistent failure'),{code:'EPERM'});};
    assert.throws(()=>atomicWrite(file,'not committed'),error=>error.code==='EPERM');
  }finally{fs.renameSync=original;}
  assert.equal(attempts,6);assert.equal(fs.readFileSync(file,'utf8'),'updated');assert.deepEqual(fs.readdirSync(root),['case.json']);
});
