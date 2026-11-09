import test from 'node:test';
import assert from 'node:assert/strict';
import {createWiParseI2cBus} from '../plugins/wireless-whitebox/wiparse-i2c-bus.mjs';
import {createMT5788} from '../plugins/wireless-whitebox/mt5788.mjs';

test('MT5788 goes through owned confirmed WiParse I2C jobs',async()=>{
  const commands=[];let last=0,lease=true;
  const bus=createWiParseI2cBus({checkLease:async()=>{if(!lease)throw new Error('LEASE.LOST');},executeConfirmed:async command=>{
    commands.push(command);
    if(command.DongleI2cWrite){last=Number.parseInt(command.DongleI2cWrite.data_hex,16);return {data:{detail:{response:'OK WRITE'}}};}
    const data=last===0?'88 57':last===2?'88 00':'02 00 00 00';
    return {data:{detail:{response:'OK READ '+data}}};
  }});
  const chip=createMT5788(bus);assert.equal((await chip.identify()).variant,'HW');
  assert.deepEqual(commands.map(c=>Object.keys(c)[0]),['DongleI2cWrite','DongleI2cRead','DongleI2cWrite','DongleI2cRead']);
  lease=false;await assert.rejects(chip.readRxTelemetry(),/LEASE.LOST/);assert.equal(commands.length,4);
});

test('I2C malformed, short or unconfirmed replies never become success',async()=>{
  for(const response of ['ERR nack','OK READ 01','OK WRITE',null]){
    const bus=createWiParseI2cBus({checkLease:async()=>{},executeConfirmed:async()=>({data:{detail:{response}}})});
    await assert.rejects(bus.read(0x2b,2),/I2C\.(RESPONSE|SHORT)/);
  }
  assert.throws(()=>createWiParseI2cBus({executeConfirmed:async()=>{}}),/OWNER_REQUIRED/);
});
