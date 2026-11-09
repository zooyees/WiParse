import test from 'node:test';
import assert from 'node:assert/strict';
import {latestTaskQueue} from '../src/latestTaskQueue.ts';
const flush=async()=>{for(let i=0;i<12;i++)await Promise.resolve();};
function deferred(){let resolve,reject;const promise=new Promise((a,b)=>{resolve=a;reject=b;});return {promise,resolve,reject};}
function fakeClock(){let time=0,id=0;const timers=new Map();return {timers,api:{now:()=>time,set:(callback,delay)=>{timers.set(++id,{callback,at:time+delay});return id;},clear:id=>timers.delete(id)},advance(ms){const end=time+ms;for(;;){const due=[...timers].filter(([,job])=>job.at<=end).sort((a,b)=>a[1].at-b[1].at)[0];if(!due)break;time=due[1].at;timers.delete(due[0]);due[1].callback();}time=end;}};}

test('continuous input advances at fixed intervals instead of restarting an idle debounce',async()=>{
 const clock=fakeClock(),started=[],accepted=[],jobs=[];
 const queue=latestTaskQueue(input=>{started.push({input,at:clock.api.now()});const job=deferred();jobs.push(job);return job.promise;},(output,input)=>accepted.push(input),assert.fail,40,clock.api);
 queue.submit(0);await flush();jobs[0].resolve('first');await flush();
 clock.advance(10);queue.submit(1);clock.advance(10);queue.submit(2);clock.advance(10);queue.submit(3);
 assert.equal(clock.timers.size,1);clock.advance(10);await flush();
 assert.deepEqual(started,[{input:0,at:0},{input:3,at:40}]);
 // A newer target must not suppress all successful intermediate progress.
 queue.submit(4);jobs[1].resolve('intermediate');await flush();assert.deepEqual(accepted,[0,3]);
 clock.advance(40);await flush();assert.equal(started.at(-1).input,4);jobs[2].resolve('last');await flush();assert.deepEqual(accepted,[0,3,4]);queue.dispose();
});

test('slow jobs stay serial with one latest pending target and invalidated sources never publish',async()=>{
 const clock=fakeClock(),jobs=[],accepted=[],errors=[];let active=0,maximum=0;
 const queue=latestTaskQueue(async(input,alive)=>{active++;maximum=Math.max(maximum,active);const job={input,alive,...deferred()};jobs.push(job);try{return await job.promise;}finally{active--; }},(output,input)=>accepted.push(input),(error,input)=>errors.push(input),40,clock.api);
 queue.submit('old');await flush();for(let i=0;i<1000;i++)queue.submit('obsolete'+i);
 queue.invalidate();queue.submit('new1');queue.submit('new2');assert.equal(jobs[0].alive(),false);
 await flush();assert.equal(jobs.length,1);jobs[0].resolve('obsolete result');await flush();
 assert.deepEqual(accepted,[]);assert.equal(jobs.length,2);assert.equal(jobs[1].input,'new2');assert.equal(jobs[1].alive(),true);
 jobs[1].resolve('current result');await flush();assert.deepEqual(accepted,['new2']);assert.deepEqual(errors,[]);assert.equal(maximum,1);queue.dispose();
});

test('superseded failures are quiet while the latest failure is reported and later input recovers',async()=>{
 const clock=fakeClock(),jobs=[],errors=[],accepted=[];
 const queue=latestTaskQueue(input=>{const job={input,...deferred()};jobs.push(job);return job.promise;},(output,input)=>accepted.push(input),(error,input)=>errors.push([input,String(error)]),40,clock.api);
 queue.submit('a');await flush();queue.submit('b');jobs[0].reject(Error('old failure'));await flush();assert.deepEqual(errors,[]);
 clock.advance(40);await flush();jobs[1].reject(Error('latest failure'));await flush();assert.deepEqual(errors,[['b','Error: latest failure']]);
 queue.submit('c');clock.advance(40);await flush();jobs[2].resolve('ok');await flush();assert.deepEqual(accepted,['c']);queue.dispose();
});

test('cleanup prevents not-yet-started work and cancels pending timers',async()=>{
 const clock=fakeClock();let calls=0;
 const queue=latestTaskQueue(async()=>{calls++;return 1;},assert.fail,assert.fail,40,clock.api);
 queue.submit(1);queue.dispose();await flush();assert.equal(calls,0);
 const second=latestTaskQueue(async()=>++calls,()=>{},assert.fail,40,clock.api);
 second.submit(1);await flush();second.submit(2);assert.equal(clock.timers.size,1);
 const stale=[...clock.timers.values()][0].callback;second.dispose();assert.equal(clock.timers.size,0);stale();second.submit(3);await flush();assert.equal(calls,1);
});
