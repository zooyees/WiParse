import test from 'node:test';
import assert from 'node:assert/strict';
import {frameScheduler} from '../src/frameScheduler.ts';
test('resize bursts coalesce into one pending draw and later frames still render',()=>{
 const jobs=new Map();let next=0,draws=0;
 const scheduler=frameScheduler(()=>draws++,callback=>{jobs.set(++next,callback);return next;},id=>jobs.delete(id));
 for(let i=0;i<100;i++)scheduler.schedule();assert.equal(jobs.size,1);
 const first=jobs.get(1);jobs.delete(1);first();assert.equal(draws,1);
 scheduler.schedule();assert.equal(jobs.size,1);jobs.get(2)();assert.equal(draws,2);
});
test('dispose cancels scheduled work and rejects stale callbacks after cleanup',()=>{
 let saved,cancelled,draws=0;const scheduler=frameScheduler(()=>draws++,callback=>{saved=callback;return 7;},id=>{cancelled=id;});
 scheduler.schedule();scheduler.dispose();assert.equal(cancelled,7);saved();scheduler.schedule();assert.equal(draws,0);
});
