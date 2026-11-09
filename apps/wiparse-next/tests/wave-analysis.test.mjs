import test from 'node:test';import assert from 'node:assert/strict';
import {protocolGate,cursorFocusWindow,overlayBuckets,filterFrames,stepFrame,frameFocusWindow,zoomVertical,cursorFrequency,reconcileChannels,visibleExtent} from '../src/waveAnalysis.ts';
const frames=[{index:0,t_start:0,t_end:1,summary:'ACK',preview:'AA 55',error:'none',bytes:2},{index:1,t_start:1,t_end:2,summary:'DATA',preview:'33 CC',error:'checksum',bytes:2},{index:2,t_start:2,t_end:3,summary:'DATA',preview:'AA 01',error:'none',bytes:2}];
test('protocol search and issue-only navigation share exact original frame identities',()=>{assert.deepEqual(filterFrames(frames,'aa',false).map(f=>f.index),[0,2]);assert.deepEqual(filterFrames(frames,'data',true).map(f=>f.index),[1]);assert.equal(stepFrame(filterFrames(frames,'',true),null,1),1);assert.equal(stepFrame(frames,null,-1),2);assert.equal(stepFrame([],0,1),null);assert.equal(stepFrame(frames,2,1),2);});
test('vertical zoom anchors the value under the pointer and rejects invalid extents',()=>{assert.deepEqual(zoomVertical({minimum:0,maximum:100},.25,.5),{minimum:37.5,maximum:87.5});assert.equal(zoomVertical({minimum:0,maximum:1},.5,NaN),null);assert.equal(zoomVertical({minimum:2,maximum:1},.5,.5),null);assert.equal(zoomVertical({minimum:-Number.MAX_VALUE,maximum:Number.MAX_VALUE},.5,2),null);});
test('cursor frequency uses explicit time units and never labels arbitrary units as Hz',()=>{assert.equal(cursorFrequency(0,2,'ms'),500);assert.equal(cursorFrequency(0,2,'s'),.5);assert.equal(cursorFrequency(0,2,'samples'),null);assert.equal(cursorFrequency(1,1,'s'),null);});
const source=(files)=>({files:files.map(([id,channels])=>({id,channels})),channels:files.flatMap(([,ids])=>ids.map(index=>({index,x_unit:'s',start:index*10,end:index*10+1,points:101})))});
test('new multi-channel documents start visible, selection and index remaps preserve hidden channels',()=>{const a=source([['a',[0,1]],['b',[2,3]]]);assert.deepEqual(reconcileChannels(null,a,0,[]),{primary:0,overlays:[1,2,3]});const b=source([['b',[0,1]]]);assert.deepEqual(reconcileChannels(a,b,2,[0,3]),{primary:0,overlays:[1]});assert.deepEqual(reconcileChannels(a,a,0,[2]),{primary:0,overlays:[2]});const c=source([['a',[0,1]],['b',[2,3]],['c',[4]]]);assert.deepEqual(reconcileChannels(a,c,0,[2]),{primary:0,overlays:[2,4]});});
test('incompatible X units are excluded and full fit covers disjoint visible sources',()=>{const a=source([['a',[0,1]]]);a.channels[1].x_unit='ms';assert.deepEqual(reconcileChannels(null,a,0,[]).overlays,[]);const extent=visibleExtent(a.channels,[0,1]);assert.equal(extent.start,0);assert.equal(extent.end,11);assert.equal(extent.points,1101);assert.equal(visibleExtent(a.channels,[]),null);});

test('hex search covers payload beyond the visible preview and constant-time channels remain navigable',()=>{assert.equal(filterFrames([{...frames[0],hex:'AA 55 33 CC DE AD BE EF'}],'DE AD',false).length,1);const extent=visibleExtent([{index:0,start:2,end:2,points:1}],[0]);assert.equal(extent.points,2);assert.ok(Number.isFinite(extent.points));});

test('short transactions in long captures get a transaction-sized focus window',()=>{const next=frameFocusWindow({t_start:500,t_end:500.00001},{start:0,end:1000,points:1000000001});assert(next.end-next.start<.001);assert(next.start<500&&next.end>500.00001);assert.equal(frameFocusWindow({t_start:2,t_end:1},{start:0,end:3,points:100}),null);});

test('overlay requests respect native per-channel limit and aggregate display budget',()=>{assert.equal(overlayBuckets(1),2048);assert.equal(overlayBuckets(2),2048);assert.equal(overlayBuckets(3),1400);assert.equal(overlayBuckets(1000),32);assert.equal(overlayBuckets(0),2048);});

test('protocol cursor gates use exact raw boundaries without display padding',()=>{
 for(const kind of ['uart','i2c','spi','i2s','usb_pd','scp','ufcs']){
  assert.deepEqual(protocolGate(kind,true,'cursors',[{x:2},{x:1}],{start:0,end:10}),{start:1,end:2});
  assert.deepEqual(protocolGate(kind,true,'cursors',[{x:1},{x:1}],null),{start:1,end:1});
  assert.equal(protocolGate(kind,true,'cursors',[{x:1}],null),null);
 }
});
test('DDSSS measurement cursors leave correlator acquisition intact; viewport crop is explicit',()=>{
 const view={start:2,end:3},cursors=[{x:2.2},{x:2.3}];
 assert.equal(protocolGate('ddsss',true,'cursors',cursors,view),null);
 assert.deepEqual(protocolGate('ddsss',true,'view',cursors,view),view);
 assert.equal(protocolGate('ddsss',false,'view',cursors,view),null);
 assert.equal(protocolGate('uart',false,'cursors',cursors,view),null);
 assert.equal(protocolGate('uart',true,'view',cursors,null),null);
});
test('cursor display focus stays within document edges and source resolution',()=>{
 const source={start:0,end:100,points:1001};
 assert.deepEqual(cursorFocusWindow([{x:0},{x:10}],source),{start:0,end:11});
 assert.deepEqual(cursorFocusWindow([{x:90},{x:100}],source),{start:89,end:100});
 const narrow=cursorFocusWindow([{x:50},{x:50.00001}],source);assert(narrow.end-narrow.start>=.2-1e-12);
 assert.equal(cursorFocusWindow([{x:1},{x:1}],source),null);
 assert.equal(cursorFocusWindow([{x:1}],source),null);
});
