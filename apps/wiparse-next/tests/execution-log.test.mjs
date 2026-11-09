import test from 'node:test';
import assert from 'node:assert/strict';
import {applyExecutionPage,emptyExecutionText} from '../src/executionLog.ts';
const size=text=>new TextEncoder().encode(text).byteLength;
const page=(text,from=0,extra={})=>({available:true,project:'project-a',plugin:'plugin-a',execution_id:'job-a',from,next:from+size(text),total:from+size(text),dropped:0,text,...extra});
test('execution pages use UTF-8 byte offsets and append only contiguous chunks',()=>{
 let state=applyExecutionPage(emptyExecutionText(),page('日志 😀\n'),'project-a','plugin-a');
 assert.equal(state.cursor,size('日志 😀\n'));
 state=applyExecutionPage(state,page('done\n',state.cursor),'project-a','plugin-a');assert.equal(state.text,'日志 😀\ndone\n');
 assert.throws(()=>applyExecutionPage(state,page('bad',0),'project-a','plugin-a'),/连续|长度/);
 assert.throws(()=>applyExecutionPage(emptyExecutionText(),page('😀',0,{next:2,total:2}),'project-a','plugin-a'),/连续|长度/);
});
test('job changes, missing sessions, and foreign sources cannot retain another execution text',()=>{
 const state=applyExecutionPage(emptyExecutionText(),page('old'),'project-a','plugin-a');
 assert.throws(()=>applyExecutionPage(state,page('foreign',3,{project:'project-b'}),'project-a','plugin-a'),/来源/);
 const changed=applyExecutionPage(state,{available:true,project:'project-a',plugin:'plugin-a',execution_id:'job-b',changed:true,dropped:12},'project-a','plugin-a');
 assert.equal(changed.text,'');assert.equal(changed.cursor,12);assert.equal(changed.id,'job-b');
 assert.deepEqual(applyExecutionPage(state,{available:false},'project-a','plugin-a'),emptyExecutionText());
});
test('ring overflow explicitly marks a gap instead of silently joining missing context',()=>{
 const state=applyExecutionPage(emptyExecutionText(),page('old'),'project-a','plugin-a');
 const next=applyExecutionPage(state,page('new',100,{dropped:100}),'project-a','plugin-a');
 assert.ok(next.text.includes('保留窗口已前移'));assert.ok(!next.text.includes('old'));assert.ok(next.text.endsWith('new'));
});
