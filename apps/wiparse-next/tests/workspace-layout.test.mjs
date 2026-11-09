import test from 'node:test';
import assert from 'node:assert/strict';
import {railWidth,railMaximum,restoreRail} from '../src/workspaceLayout.ts';
test('rail keeps 200px minimum and reserves room for content at supported window sizes',()=>{
  for(const container of [760,1100,1440,1920])for(const preferred of [0,200,350,480,10000,NaN,Infinity]){
    const width=railWidth(preferred,container);assert(width>=200&&width<=480);assert(container-width-6>=240);
  }
  assert.equal(railWidth(480,560),314);
});
test('narrow containers keep the rail readable and require overflow instead of hiding controls',()=>{
  for(const width of [0,180,320,440]){assert.equal(railMaximum(width),200);assert.equal(railWidth(480,width),200);}
});
test('corrupt or obsolete persisted widths recover to the user-required default',()=>{
  for(const value of [null,'','undefined','NaN','Infinity','-1','199','481','{"width":300}'])assert.equal(restoreRail(value),200);
  assert.equal(restoreRail('320'),320);assert.equal(restoreRail('480'),480);
});
