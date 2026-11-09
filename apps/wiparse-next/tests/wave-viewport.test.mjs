import test from 'node:test';
import assert from 'node:assert/strict';
import {readWaveViewport} from '../src/waveViewport.ts';
const flush=async()=>{for(let i=0;i<12;i++)await Promise.resolve();};
function deferred(){let resolve,reject;const promise=new Promise((a,b)=>{resolve=a;reject=b;});return {promise,resolve,reject};}
const view=channel=>({channel:'CH'+channel,x_unit:'s',y_unit:'V',start:2,end:3,minimum:0,maximum:1,samples:10,buckets:[]});
const request={channel:0,overlays:[1,2,3,4,5,6],start:2,end:3};

test('a viewport publishes all channels together, with at most three reads and stable identities',async()=>{
 const jobs=[];let active=0,maximum=0,settled=false;
 const promise=readWaveViewport({...request,overlays:[1,2,3,4,5,6,1,0]},async input=>{active++;maximum=Math.max(maximum,active);const job={input,...deferred()};jobs.push(job);try{return await job.promise;}finally{active--;}}).then(value=>{settled=true;return value;});
 assert.equal(jobs.length,3);assert.equal(settled,false);
 for(let i=0;i<3;i++){const wave=jobs.filter(job=>!job.done).reverse();for(const job of wave){job.done=true;job.resolve(view(job.input.channel));}await flush();if(jobs.some(job=>!job.done))assert.equal(settled,false);}
 const result=await promise;assert.equal(maximum,3);assert.equal(jobs.length,7);
 assert.equal(result.view.channelIndex,0);assert.deepEqual(result.overlays.map(item=>item.channelIndex),[1,2,3,4,5,6]);assert.deepEqual(result.failures,[]);
 for(const job of jobs){assert.equal(job.input.start,2);assert.equal(job.input.end,3);assert(job.input.buckets>=32&&job.input.buckets<=2048);}
});

test('a primary failure stops queued reads but waits for active workers before the next batch',async()=>{
 const jobs=[];let rejected=false;
 const promise=readWaveViewport({...request,overlays:[1,2,3,4,5,6]},input=>{const job={input,...deferred()};jobs.push(job);return job.promise;});
 const assertion=assert.rejects(promise,/primary failed/).then(()=>{rejected=true;});
 jobs[0].reject(Error('primary failed'));await flush();assert.equal(rejected,false);
 jobs[1].resolve(view(1));await flush();assert.equal(rejected,false);assert.equal(jobs.length,3);jobs[2].resolve(view(2));await assertion;assert.equal(jobs.length,3);
});

test('failed overlays retain other curves and identify the failed channel',async()=>{
 const result=await readWaveViewport({...request,overlays:[1,2]},async input=>{if(input.channel===1)throw Error('read failed');return view(input.channel);});
 assert.equal(result.view.channelIndex,0);assert.deepEqual(result.overlays.map(item=>item.channelIndex),[2]);assert.deepEqual(result.failures,[{channel:1,reason:'Error: read failed'}]);
 const empty=await readWaveViewport({...request,overlays:[1]},async input=>({...view(input.channel),samples:0}));assert.equal(empty.view.samples,0);assert.equal(empty.overlays.length,1);assert.deepEqual(empty.failures,[]);
});

test('source invalidation stops launching more native reads and never returns an incomplete batch',async()=>{
 let active=true;const jobs=[];
 const promise=readWaveViewport({...request,overlays:Array.from({length:100},(_,i)=>i+1)},input=>{const job={input,...deferred()};jobs.push(job);return job.promise;},()=>active);
 const assertion=assert.rejects(promise,/source changed/);assert.equal(jobs.length,3);active=false;
 for(const job of jobs)job.resolve(view(job.input.channel));await assertion;assert.equal(jobs.length,3);
});
