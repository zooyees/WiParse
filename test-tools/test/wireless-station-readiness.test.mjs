import test from 'node:test';
import assert from 'node:assert/strict';
import {inspectStation,serialPortsFromReply} from '../plugins/wireless-whitebox/station-readiness.mjs';

test('readiness uses exact VISA identities and never sends an output command',async()=>{
  const calls=[];
  const ctx={projectConfig:{value:{station:{scope_resource:'USB::SCOPE',power_resource:'USB::POWER',load_resource:'USB::LOAD',rx_i2c_interface:'bridge://ft4222/serial=X',log_source:'COM3'}}},invoke:async(method)=>{
    calls.push(method);
    if(method==='instrument.list')return {data:{devices:[{resource:'USB::SCOPE',kind:'oscilloscope',identity:{model:'MDO3014'}},{resource:'USB::POWER',kind:'dc_source',identity:{model:'DP832'}},{resource:'USB::LOAD',kind:'electronic_load'},{resource:'bridge://ft4222/serial=X',kind:'usb_bridge'}]}};
    if(method==='serial.ports')return {ok:true,data:[{device:'COM3'}]};
    throw new Error('unexpected device command');
  }};
  const result=await inspectStation(ctx);
  assert.deepEqual(calls,['instrument.list','serial.ports']);assert.equal(result.ok,false);
  for(const code of ['SCOPE.BINDING','POWER.BINDING','UART.BINDING','LOAD.BINDING','RX.FT4222_BINDING'])assert.equal(result.checks.find(c=>c.code===code).ok,true);
  for(const code of ['RX.I2C_CONTRACT','CAPTURE.RAW_TIMING','WIRELESS.LIVE_BACKEND_NOT_VALIDATED'])assert.equal(result.checks.find(c=>c.code===code).ok,false);
  ctx.projectConfig.value.station.scope_resource='USB::WRONG';
  assert.equal((await inspectStation(ctx)).checks.find(c=>c.code==='SCOPE.BINDING').ok,false);
});

test('serial inventory accepts actual and legacy envelopes but rejects unknown responses',()=>{
  assert.deepEqual(serialPortsFromReply({ok:true,data:[{device:'COM3'}]}),[{device:'COM3'}]);
  assert.deepEqual(serialPortsFromReply({data:{ports:[{device:'COM4'}]}}),[{device:'COM4'}]);
  assert.throws(()=>serialPortsFromReply({ok:false,data:[]}),/serial request failed/);
  assert.throws(()=>serialPortsFromReply({data:{}}),/invalid serial response/);
});
