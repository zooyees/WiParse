import test from 'node:test';
import assert from 'node:assert/strict';
import {SERIAL_FRAME_MS,pausesSerialFollow,readSerialPage} from '../src/serialLive.ts';

test('only explicit upward reading gestures pause serial follow; font zoom and layout do not',()=>{
 assert.equal(pausesSerialFollow({kind:'wheel',deltaY:-1,control:false,meta:false}),true);
 assert.equal(pausesSerialFollow({kind:'wheel',deltaY:-1,control:true,meta:false}),false);
 assert.equal(pausesSerialFollow({kind:'wheel',deltaY:1,control:false,meta:false}),false);
 for(const key of ['PageUp','Home','ArrowUp'])assert.equal(pausesSerialFollow({kind:'key',key}),true);
 assert.equal(pausesSerialFollow({kind:'key',key:'ArrowDown'}),false);
 assert.equal(pausesSerialFollow({kind:'pointer',button:0,x:994,right:1000,scrollbar:12}),true);
 assert.equal(pausesSerialFollow({kind:'pointer',button:0,x:900,right:1000,scrollbar:12}),false);
 assert.equal(pausesSerialFollow({kind:'pointer',button:0,x:1000,right:1000,scrollbar:0}),false);
 assert.equal(SERIAL_FRAME_MS,33);
});

test('serial live pages advance past FIFO eviction while historical pages stay at their requested row',async()=>{
 const calls=[];let end=2000;
 const read=async params=>{calls.push(params);const from=Math.max(end-500,params.from_row);return {total:end,dropped:end-500,next_row:Math.min(end,from+500),lines:Array.from({length:Math.min(500,end-from)},(_,i)=>'row '+(from+i))};};
 let page=await readSerialPage(read,0,42,500,null,()=>true);assert.equal(page.lines.at(-1),'row 1999');
 end=2400;page=await readSerialPage(read,0,42,500,null,()=>true);assert.equal(page.lines.at(-1),'row 2399');
 assert(calls.every(p=>p.limit===500&&p.view_id===42));
 calls.length=0;await readSerialPage(read,0,42,500,1950,()=>true);assert.equal(calls.length,1);assert.equal(calls[0].from_row,1950);
});

test('serial tail corrects retained count to absolute coordinates and cancels before a second stale read',async()=>{
 const calls=[];let alive=true;
 const read=async params=>{calls.push(params);return {total:1200,next_row:params.from_row+500,lines:['x']};};
 await readSerialPage(read,2,7,500,null,()=>alive);assert.deepEqual(calls.map(p=>p.from_row),[0,700]);
 calls.length=0;const page=await readSerialPage(async p=>{alive=false;return read(p);},2,7,500,null,()=>alive);assert.equal(page,null);assert.equal(calls.length,1);
});
