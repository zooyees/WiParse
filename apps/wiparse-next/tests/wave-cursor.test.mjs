import test from 'node:test';
import assert from 'node:assert/strict';
import {cursorAxisOnSpace, cursorWindow, nextCursorPair, plotExtent, pointerY, verticalMargin, waveWindow, zoomWaveWindow, nearestCursorIndex, waveBoxZoom} from '../src/waveCursor.ts';

test('a third cursor replaces the pair and vertical clicks use the plot scale', () => {
  assert.deepEqual(nextCursorPair([], 1), [1]);
  assert.deepEqual(nextCursorPair([1], 2), [1, 2]);
  assert.deepEqual(nextCursorPair([1, 2], 3), [3]);
  assert.equal(pointerY(10, 10, 100, 0, 10), 10);
  assert.equal(pointerY(110, 10, 100, 0, 10), 0);
  assert.equal(pointerY(60, 10, 100, 0, 10), 5);
  assert.equal(pointerY(9, 10, 100, 0, 10), null);
  assert.equal(pointerY(20, 10, 100, 2, 2), null);
  assert.deepEqual(plotExtent({minimum: 1, maximum: 1}, []), {minimum: 1 - 0.05, maximum: 1.05});
  assert.equal(plotExtent({minimum: Number.NaN, maximum: 1}, []), null);
  assert.equal(cursorAxisOnSpace(' ', false, false), true);
  assert.equal(cursorAxisOnSpace(' ', true, false), false);
  assert.equal(cursorAxisOnSpace(' ', false, true), false);
  assert.deepEqual(cursorWindow(10, 0), {start: -0.5, end: 10.5});
  assert.equal(cursorWindow(3, 3), null);
  assert.deepEqual(verticalMargin(0, 100), {minimum: -8, maximum: 108});
});

test('auto extent keeps square-wave plateaus inside the plot and overlays share its cursor scale',()=>{
  const extent=plotExtent({minimum:0,maximum:5},[{minimum:-1,maximum:3.3}]);
  assert.deepEqual(extent,{minimum:-1.48,maximum:5.48});
  assert.equal(pointerY(60,10,100,extent.minimum,extent.maximum),2);
  assert.equal(plotExtent({minimum:-Number.MAX_VALUE,maximum:Number.MAX_VALUE},[]),null);
});

test("nonrepresentable padded extents are rejected instead of drawing NaN pixels",()=>{
  assert.equal(verticalMargin(0,Number.MAX_VALUE),null);
  assert.equal(verticalMargin(1e300,1e300),null);
  assert.equal(plotExtent({minimum:0,maximum:Number.MAX_VALUE},[]),null);
});

test('wheel zoom keeps the pointer sample anchored and clamps at both source ends',()=>{
  const source={start:0,end:100,points:1001};
  assert.deepEqual(zoomWaveWindow({start:20,end:60},.25,.5,source),{start:25,end:45});
  assert.deepEqual(zoomWaveWindow({start:0,end:20},0,10,source),{start:0,end:100});
  assert.deepEqual(waveWindow(-20,20,0,100,1001),{start:0,end:40});
  assert.deepEqual(waveWindow(90,130,0,100,1001),{start:60,end:100});
});
test('repeated zoom cannot collapse below source resolution or produce invalid ranges',()=>{
  const source={start:0,end:1,points:1001};let range={start:0,end:1};
  for(let i=0;i<200;i++)range=zoomWaveWindow(range,.5,.5,source);
  assert(range.end>range.start);assert(range.end-range.start>=.002-1e-15);
  assert.equal(zoomWaveWindow(range,.5,Number.NaN,source),null);
  assert.equal(waveWindow(0,1,0,0,1),null);
});

test('cursor dragging picks the closest visible handle and keeps stable pair indices',()=>{
  assert.equal(nearestCursorIndex([100,200],196),1);
  assert.equal(nearestCursorIndex([100,200],120),null);
  assert.equal(nearestCursorIndex([100,100],100),0);
  assert.equal(nearestCursorIndex([Number.NaN,200],200),1);
});
test('reverse box gestures use the same source and vertical mapping as the plot',()=>{
  const extent={minimum:-1,maximum:9};
  assert.deepEqual(waveBoxZoom(0,100,extent,{x:.8,y:.8},{x:.2,y:.2},1000,500),{range:{start:20,end:80},extent:{minimum:1,maximum:7}});
  assert.deepEqual(waveBoxZoom(0,100,extent,{x:.2,y:.2},{x:.201,y:.8},1000,500),{range:null,extent:{minimum:1,maximum:7}});
  assert.equal(waveBoxZoom(0,100,extent,{x:.2,y:.2},{x:.201,y:.201},1000,500),null);
  assert.equal(waveBoxZoom(0,100,extent,{x:NaN,y:.2},{x:.8,y:.8},1000,500),null);
});
