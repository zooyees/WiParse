import test from 'node:test';import assert from 'node:assert/strict';
import {selectedPluginFromHost} from '../src/pluginSelection.ts';
test('workspace selection follows actual host activation and never invents a default',()=>{
 const installed=[{id:'example-smoke'},{id:'fpga-firmware-test'}];
 assert.equal(selectedPluginFromHost({test_tool:{selected:null}},installed),'');
 assert.equal(selectedPluginFromHost({test_tool:{selected:'example-smoke'}},installed),'example-smoke');
 assert.equal(selectedPluginFromHost({test_tool:{selected:'removed-plugin'}},installed),'');
 assert.equal(selectedPluginFromHost({test_tool:{selected:'fpga-firmware-test'}},installed),'fpga-firmware-test');
});
