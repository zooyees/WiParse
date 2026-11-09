import test from 'node:test';
import assert from 'node:assert/strict';
import {ImageLoader,ImageRequestQueue} from '../src/reportImageLoader.ts';
const flush=()=>new Promise(resolve=>setImmediate(resolve));
const png='data:image/png;base64,AA==';

test('deduplicates images and bounds native concurrency across a report',async()=>{
  const queue=new ImageRequestQueue(2),pending=[],calls=[];
  const loader=new ImageLoader(7,(session,source)=>new Promise(resolve=>{calls.push([session,source]);pending.push(resolve);}),queue);
  const first=loader.load('one.png');assert.equal(loader.load('one.png'),first);
  const tasks=[first,loader.load('two.png'),loader.load('three.png')];
  await flush();assert.equal(calls.length,2);
  pending.shift()(png);await flush();assert.equal(calls.length,3);
  pending.splice(0).forEach(resolve=>resolve(png));await Promise.all(tasks);
  assert.deepEqual(calls.map(call=>call[0]),[7,7,7]);
});

test('cancels queued old-report images without blocking a new report',async()=>{
  const queue=new ImageRequestQueue(1),calls=[];let release;
  const old=new ImageLoader(1,(session,source)=>{calls.push([session,source]);return new Promise(resolve=>{release=resolve;});},queue);
  const oldTasks=Promise.allSettled([old.load('a'),old.load('b'),old.load('c')]);
  await flush();old.cancelPending();
  const next=new ImageLoader(2,async(session,source)=>{calls.push([session,source]);return png;},queue);
  const current=next.load('new');release(png);await current;
  const results=await oldTasks;
  assert.deepEqual(results.map(result=>result.status),['fulfilled','rejected','rejected']);
  assert.deepEqual(calls,[[1,'a'],[2,'new']]);
});

test('effect replay can reuse the loader and does not submit the cancelled work',async()=>{
  let reads=0;const loader=new ImageLoader(3,async()=>{reads++;return png;},new ImageRequestQueue(1));
  const discarded=loader.load('same').catch(reason=>reason.message);
  loader.cancelPending();const replay=loader.load('same');
  assert.match(await discarded,/取消/);assert.equal(await replay,png);assert.equal(reads,1);
});

test('errors release queue slots and malformed responses produce a visible error',async()=>{
  const loader=new ImageLoader(4,async(_session,source)=>{if(source==='bad')throw Error('failed');return source==='invalid'?'not an image':png;},new ImageRequestQueue(1));
  const results=await Promise.allSettled([loader.load('bad'),loader.load('invalid'),loader.load('good')]);
  assert.equal(results[0].status,'rejected');assert.match(results[1].reason.message,/格式无效/);assert.equal(results[2].value,png);
});
