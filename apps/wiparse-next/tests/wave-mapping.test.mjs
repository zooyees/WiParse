import {axisReadoutExtent} from '../src/waveMapping.ts';
import test from 'node:test';
import assert from 'node:assert/strict';
import {identityMapping,overlayCompatible,stackMappings,displayValue,sourceValue,validMapping,measurementWindow} from '../src/waveMapping.ts';
test('display gain and ground offset are invertible without changing source samples',()=>{
 const source=Object.freeze([-2,0,1.25,5]);const mapping={scale:2.5,offset:-3};
 assert.deepEqual(source.map(value=>sourceValue(displayValue(value,mapping),mapping)),source);
 assert.deepEqual(source.map(value=>displayValue(value,identityMapping)),source);
 assert.equal(validMapping({scale:0,offset:0}),false);assert.equal(validMapping({scale:Infinity,offset:0}),false);assert.equal(validMapping({scale:1,offset:NaN}),false);
});
test('measurement windows use raw time and normalize reversed cursor order',()=>{
 const full={start:0,end:10},view={start:3,end:7};
 assert.deepEqual(measurementWindow('auto',full,view,[{x:6},{x:4}]),{start:4,end:6});
 assert.deepEqual(measurementWindow('full',full,view,[{x:4},{x:6}]),full);
 assert.deepEqual(measurementWindow('view',full,view,[]),view);
 assert.deepEqual(measurementWindow('auto',full,view,[{x:4}]),full);
});

test('automatic stacking separates mixed-unit channels and keeps original values invertible',()=>{
 const channels=[{index:8,minimum:4000,maximum:6000},{index:3,minimum:.004,maximum:.006},{index:21,minimum:2,maximum:2}];const mappings=stackMappings(channels);assert.ok(mappings);
 channels.forEach((channel,slot)=>{const center=channel.minimum+(channel.maximum-channel.minimum)/2;assert.ok(Math.abs(displayValue(center,mappings[channel.index])+slot*3)<1e-10);assert.ok(Math.abs(sourceValue(displayValue(channel.maximum,mappings[channel.index]),mappings[channel.index])-channel.maximum)<1e-10);});
 assert.equal(stackMappings([{index:0,minimum:NaN,maximum:1}]),null);
});

test('overlay compatibility follows the horizontal axis and permits independent Y units',()=>{
 const voltage={x_unit:'s',y_unit:'V'},current={x_unit:'s',y_unit:'A'};
 assert.equal(overlayCompatible(current,voltage),true);assert.equal(overlayCompatible({x_unit:'ms'},voltage),false);assert.equal(overlayCompatible(voltage,undefined),false);
});

test('selected channel axis inverts scale and offset without relabelling other channel data',()=>{assert.deepEqual(axisReadoutExtent({minimum:1,maximum:11},{scale:2,offset:1}),{minimum:0,maximum:5,original:true});assert.equal(axisReadoutExtent({minimum:-1e300,maximum:1e300},{scale:1e-12,offset:0}).original,false);assert.equal(axisReadoutExtent({minimum:0,maximum:1},{scale:0,offset:0}).original,false);});
