import test from 'node:test';
import assert from 'node:assert/strict';
import {applyLanguage,getLanguage,localizeLabel,subscribeLanguage} from '../src/language.ts';
test('interface language switches without changing unknown source data',()=>{
  let changes=0;const stop=subscribeLanguage(()=>changes++);
  applyLanguage('en-US');assert.equal(getLanguage(),'en');assert.equal(localizeLabel('运行'),'Run');
  assert.equal(localizeLabel('UART RX: 用户原文 0xAA'),'UART RX: 用户原文 0xAA');
  applyLanguage('en');assert.equal(changes,1);applyLanguage('zh');assert.equal(localizeLabel('运行'),'运行');assert.equal(changes,2);stop();
});
