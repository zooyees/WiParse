import test from 'node:test';
import assert from 'node:assert/strict';
import {axisScale,coordinateReadout} from '../src/plotAxes.ts';
test('small signals use a shared exponent rather than clipped long decimals',()=>{
 const axis=axisScale(-0.00002504326,0.00002504326);
 assert.equal(axis.annotation,'×10^-6');assert.equal(axis.labels[2],'0');
 assert.ok(axis.labels.every(value=>value.length<=9));
 for(let i=0;i<5;i++)assert.ok(Math.abs(Number(axis.labels[i])*1e-6-axis.values[i])<1e-8);
});
test('large offset with a small span keeps distinct tick labels and explicit origin',()=>{
 const axis=axisScale(1e12,1e12+0.01);assert.ok(axis.annotation.includes('offset 1000000000000'));
 assert.equal(new Set(axis.labels).size,5);assert.equal(axis.values[0],1e12);assert.equal(axis.values[4],1e12+0.01);
});
test('ordinary, zero and extreme finite axes remain valid',()=>{
 assert.deepEqual(axisScale(0,1).labels,['0','0.25','0.5','0.75','1']);
 assert.ok(axisScale(-1,1).labels.every(value=>!value.startsWith('-0.0')));
 assert.deepEqual(axisScale(0,0).labels,['0','0','0','0','0']);
 for(const [a,b] of [[1e-310,5e-310],[-1e308,1e308],[1000,2000]]){
  const axis=axisScale(a,b);assert.ok(axis.values.every(Number.isFinite));assert.ok(axis.labels.every(value=>Number.isFinite(Number(value))));
 }
 assert.throws(()=>axisScale(NaN,1));assert.throws(()=>axisScale(2,1));
});

test('absolute viewport and cursor readouts retain the distinction hidden by seven digits',()=>{
 const a=1e9,b=1e9+0.000499,span=b-a;
 assert.notEqual(coordinateReadout(a,span),coordinateReadout(b,span));
 assert.equal(Number(coordinateReadout(b,span).replaceAll(',','')),b);
 assert.notEqual(coordinateReadout(1e12,0.01),coordinateReadout(1e12+0.001,0.01));
});
test('ordinary readouts stay concise and zero-width/invalid values are explicit',()=>{
 assert.equal(coordinateReadout(0.29975,0.6),'0.29975');assert.equal(coordinateReadout(0,0),'0');
 assert.equal(coordinateReadout(NaN,1),'—');assert.equal(coordinateReadout(Infinity,1),'—');
 assert.equal(coordinateReadout(Number.MIN_VALUE,Number.MIN_VALUE),'0.'+'0'.repeat(323)+'5');
});
