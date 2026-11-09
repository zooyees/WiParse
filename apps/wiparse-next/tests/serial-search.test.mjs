import test from 'node:test';
import assert from 'node:assert/strict';
import {resolveLogTab,retainedRowInput,searchSerialHistory} from '../src/serialSearch.ts';

function source(lines,{dropped=0,view=7}={}) {
  return async ({view_id,content_epoch,from_row,limit})=>{
    if(view_id!==view)throw Error('LOG.VIEW_CHANGED');
    if(content_epoch!==undefined&&content_epoch!==0)throw Error('LOG.CONTENT_CHANGED');
    const start=Math.max(dropped,from_row),rows=lines.slice(start-dropped,start-dropped+limit);
    return {view_id:view,content_epoch:0,dropped,total:dropped+lines.length,truncated:from_row<dropped,lines:rows,next_row:start+rows.length};
  };
}
const pause=async()=>{};
test('history search can distinguish case or ignore it',async()=>{
  const read=source(['ALARM','alarm']);
  assert.equal((await searchSerialHistory(read,0,7,'alarm',null,()=>false,()=>{},pause)).row,0);
  assert.equal((await searchSerialHistory(read,0,7,'alarm',null,()=>false,()=>{},pause,true)).row,1);
});
test('tracks the chosen log through reordered tabs and refuses an absent view',()=>{
  const entries=[{tab_id:0,view_id:3},{tab_id:1,view_id:7}];
  assert.equal(resolveLogTab(entries,2,7,0)?.tab_id,1);
  assert.equal(resolveLogTab(entries,1,9,0),undefined);
  assert.equal(resolveLogTab(entries,null,null,1)?.view_id,7);
});
test('finds a match beyond the current page and resumes after it',async()=>{
  const rows=Array.from({length:1200},(_,i)=>i===780?'ALARM first':i===1120?'alarm second':`line ${i}`);
  const progress=[];const read=source(rows);
  assert.deepEqual(await searchSerialHistory(read,0,7,'alarm',null,()=>false,(n)=>progress.push(n),pause),{row:780,dropped:0});
  assert.deepEqual(progress,[500]);
  assert.deepEqual(await searchSerialHistory(read,0,7,'alarm',780,()=>false,()=>{},pause),{row:1120,dropped:0});
});
test('continues correctly when the byte limit makes a page shorter',async()=>{
  const rows=Array.from({length:160},(_,i)=>i===125?'target':'ordinary');const base=source(rows);
  const read=async params=>base({...params,limit:Math.min(params.limit,50)});
  assert.deepEqual(await searchSerialHistory(read,0,7,'target',null,()=>false,()=>{},pause),{row:125,dropped:0});
});
test('detects evicted rows and refuses a mixed snapshot',async()=>{
  const base=source(Array(1000).fill('no match'));let calls=0;
  const read=async params=>{const page=await base(params);if(++calls>2)page.dropped=5;return page;};
  await assert.rejects(searchSerialHistory(read,0,7,'alarm',null,()=>false,()=>{},pause),/日志内容已变化/);
});
test('rejects a replaced log even when its row count is unchanged',async()=>{
  const base=source(Array(1000).fill('no match'));let calls=0;
  const read=async params=>{const page=await base(params);if(++calls>2)page.content_epoch=1;return page;};
  await assert.rejects(searchSerialHistory(read,0,7,'alarm',null,()=>false,()=>{},pause),/日志内容已变化/);
});
test('reports previously evicted rows and honors cancellation',async()=>{
  const read=source(Array(600).fill('ok'),{dropped:50});
  assert.deepEqual(await searchSerialHistory(read,0,7,'missing',null,()=>false,()=>{},pause),{row:null,dropped:50});
  let stopped=false;
  assert.equal(await searchSerialHistory(read,0,7,'missing',null,()=>stopped,()=>{stopped=true;},pause),null);
});

test('next and previous wrap within the same retained snapshot',async()=>{
  const read=source(['alarm first','ordinary','alarm last'],{dropped:10});
  assert.deepEqual(await searchSerialHistory(read,0,7,'alarm',12,()=>false,()=>{},pause),{row:10,dropped:10});
  assert.deepEqual(await searchSerialHistory(read,0,7,'alarm',10,()=>false,()=>{},pause,false,'previous'),{row:12,dropped:10});
  assert.deepEqual(await searchSerialHistory(read,0,7,'alarm',12,()=>false,()=>{},pause,false,'previous'),{row:10,dropped:10});
});
test('reverse search finds the last match even with byte-shortened pages',async()=>{
  const rows=Array.from({length:1300},(_,i)=>i===780||i===1120?'alarm':`line ${i}`),base=source(rows);
  const read=params=>base({...params,limit:Math.min(params.limit,37)});
  assert.equal((await searchSerialHistory(read,0,7,'alarm',null,()=>false,()=>{},pause,false,'previous')).row,1120);
  assert.equal((await searchSerialHistory(read,0,7,'alarm',1120,()=>false,()=>{},pause,false,'previous')).row,780);
});
test('reverse search rejects mutation and honors cancellation between partial pages',async()=>{
  const base=source(Array(1200).fill('none'));let calls=0;
  await assert.rejects(searchSerialHistory(async params=>{const result=await base(params);if(++calls>2)result.content_epoch=9;return result;},0,7,'alarm',null,()=>false,()=>{},pause,false,'previous'),/日志内容已变化/);
  let stopped=false;
  assert.equal(await searchSerialHistory(base,0,7,'alarm',null,()=>stopped,()=>{stopped=true;},pause,false,'previous'),null);
});
test('row navigation refuses evicted rows, partial numbers and unsafe integers',()=>{
  assert.equal(retainedRowInput('51',50,100),50);
  assert.equal(retainedRowInput('100',50,100),99);
  for(const value of ['0','50','101','1.5','1e2','9007199254740993'])assert.equal(retainedRowInput(value,50,100),null);
});

test('serial navigation shortcuts preserve browser chords, repeats and direction',async()=>{
  const {serialLogShortcut}=await import('../src/serialSearch.ts');
  assert.equal(serialLogShortcut('g',true,false,false,false),'goto');
  assert.equal(serialLogShortcut('F',true,false,false,false),'find');
  assert.equal(serialLogShortcut('F3',false,false,false,false),'next');
  assert.equal(serialLogShortcut('F3',false,true,false,false),'previous');
  assert.equal(serialLogShortcut('Escape',false,false,false,false),'escape');
  for(const args of [['g',true,false,true,false],['g',true,true,false,false],['F3',true,false,false,false],['F3',false,false,false,true]])assert.equal(serialLogShortcut(...args),null);
});

test('collects matching retained rows across byte-shortened pages with bounded previews',async()=>{
  const {collectSerialMatches}=await import('../src/serialSearch.ts');
  const base=source(Array.from({length:1600},(_,i)=>i%2===0?'ALARM '+('x'.repeat(400)):'ordinary'),{dropped:27});
  const calls=[];const read=async params=>{calls.push(params.from_row);return base({...params,limit:Math.min(params.limit,71)});};
  const result=await collectSerialMatches(read,2,7,'alarm',()=>false,()=>{},pause);
  assert.equal(result.count,800);assert.equal(result.rows.length,800);assert.equal(result.rows[0].row,27);assert.equal(result.rows.at(-1).row,1625);
  assert.equal(result.rows[0].preview.length,240);assert.equal(result.limited,false);assert.equal(result.epoch,0);
  assert.ok(calls.length>20);
});

test('all-results capacity does not change the count or original log limit',async()=>{
  const {collectSerialMatches}=await import('../src/serialSearch.ts');
  const result=await collectSerialMatches(source(Array(1205).fill('ALARM')),0,7,'ALARM',()=>false,()=>{},pause,true);
  assert.equal(result.count,1205);assert.equal(result.rows.length,1000);assert.equal(result.limited,true);assert.equal(result.total,1205);
});

test('all-results scan rejects eviction, edits and cancellation instead of mixing snapshots',async()=>{
  const {collectSerialMatches}=await import('../src/serialSearch.ts');const base=source(Array(1100).fill('alarm'));
  for(const change of [{content_epoch:1},{dropped:1}]){
    let calls=0;const read=async params=>{const page=await base(params);return ++calls>2?{...page,...change}:page;};
    await assert.rejects(collectSerialMatches(read,0,7,'alarm',()=>false,()=>{},pause),/日志内容已变化/);
  }
  let cancelled=false;
  assert.equal(await collectSerialMatches(base,0,7,'alarm',()=>cancelled,()=>{cancelled=true;},pause),null);
});
