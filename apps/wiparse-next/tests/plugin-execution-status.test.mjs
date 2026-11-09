import test from 'node:test';import assert from 'node:assert/strict';
import {pluginExecutionStatus} from '../src/pluginExecutionStatus.ts';
const base={project:'a',selected:'smoke',running:false,status:'完成 · smoke'};
test('completion messages require an accepted action in the same active project and exact plugin',()=>{
 assert.equal(pluginExecutionStatus(base,'a','smoke',true),'完成 · smoke');
 assert.equal(pluginExecutionStatus(base,'a','smoke',false),'当前无运行任务');
 assert.equal(pluginExecutionStatus({...base,status:'错误 · longer-smoke'},'a','smoke',true),'当前无运行任务');
 assert.equal(pluginExecutionStatus(base,'b','smoke',true),'等待操作');
});
test('live host status distinguishes the selected plugin from another running test',()=>{
 assert.equal(pluginExecutionStatus({...base,running:true,running_plugin:'smoke'},'a','smoke',false),'任务运行中');
 assert.equal(pluginExecutionStatus({...base,running:true,running_plugin:'other'},'a','smoke',false),'其他测试运行中');
});
