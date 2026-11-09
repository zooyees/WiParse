import test from 'node:test';
import assert from 'node:assert/strict';
import {buildRows,formatTime,matchesStatus,phaseLabel,statusLabel,visibleRange} from '../src/model.ts';

test('configured items use matching results, with no fabricated pass',()=>{
  const rows=buildRows({items:[{id:'a',name:'启动'},{id:'b',name:'复位'}]},{items:[{id:'a',status:'pass',elapsed_s:1.25},{id:'other',status:'fail'}]});
  assert.equal(rows.length,2);assert.equal(rows[0].status,'pass');assert.equal(rows[1].status,'pending');assert.equal(formatTime(rows[0].elapsed),'1.3 s');
});
test('validation and historical phase use readable labels',()=>{
  const [row]=buildRows({items:[{id:'x',status:'invalid',errors:[{path:'/log_capture',keyword:'required',params:{missingProperty:'resource_role'}}]}]},{items:[{id:'x',status:'pass',phase:'finished',attempt:1}]});
  assert.equal(row.status,'invalid');assert.equal(row.attempt,1);assert.match(row.validation[0],/缺少必填字段/);
  assert.equal(phaseLabel(row.phase),'已结束');assert.equal(phaseLabel(''),'—');
});
test('status filters never count stale results as passed',()=>{
  const [invalid,stale,pending]=buildRows({items:[{id:'a',status:'invalid',sha256:'x'},{id:'b',sha256:'x'},{id:'c'}]},{items:[{id:'a',status:'pass',config_sha256:'y'},{id:'b',status:'pass',config_sha256:'y'},{id:'c',status:'tbt'}]});
  assert.equal(matchesStatus(invalid,'invalid'),true);assert.equal(matchesStatus(invalid,'stale'),false);
  assert.equal(matchesStatus(stale,'pass'),false);assert.equal(matchesStatus(stale,'stale'),true);assert.equal(matchesStatus(pending,'pending'),true);
});
test('500 test items render a bounded window and retain the correct scroll geometry',()=>{
  const range=visibleRange(500,3600,360);
  assert.ok(range.end-range.start<=19);
  assert.equal(range.before+(range.end-range.start)*36+range.after,500*36);
  assert.deepEqual(visibleRange(0,0,400),{start:0,end:0,before:0,after:0});
});
test('stale and missing results are not shown as current pass',()=>{
  assert.equal(statusLabel('pass',true),'结果已过期');assert.equal(statusLabel('running',false),'测试中');
  assert.equal(buildRows({items:[{id:'a'}]},null)[0].status,'unknown');
  const rows=buildRows({items:[{id:'a',sha256:'new'},{id:'b',status:'invalid'}]},{items:[{id:'a',status:'pass',config_sha256:'old'},{id:'b',status:'pass'}]});
  assert.equal(rows[0].stale,true);assert.equal(rows[1].status,'invalid');assert.equal(statusLabel(rows[1].status,rows[1].stale),'配置无效');assert.equal(statusLabel('pass',false),'历史通过');
  assert.equal(buildRows({items:[{id:'a'}]},{available:false})[0].status,'unknown');
  assert.equal(buildRows({items:[{id:'a'}]},{available:false,items:[{id:'a',status:'pass'}]})[0].status,'unknown');
  assert.deepEqual(buildRows({items:[]},{items:[{id:'old',status:'pass'}]}),[]);
  assert.deepEqual(buildRows(null,{items:[{id:'old',status:'pass'}]}),[]);
  assert.equal(buildRows({items:[{id:'a'}]},{error:'MATRIX.STALE_RUN',items:[{id:'a',status:'pass'}]})[0].stale,true);
});
