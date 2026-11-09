import test from 'node:test';
import assert from 'node:assert/strict';
import {createFT4222Bus} from '../plugins/wireless-whitebox/ft4222-i2c-bus.mjs';
import {createMT5788} from '../plugins/wireless-whitebox/mt5788.mjs';

test('FT4222 sends separate address writes and reads and verifies returned bytes',async()=>{
  const commands=[];let reg=0,lease=true;
  const bus=createFT4222Bus({checkLease:async()=>{if(!lease)throw new Error('LEASE.LOST');},executeConfirmed:async command=>{
    commands.push(command.BridgeI2c);const c=command.BridgeI2c;
    if(c.write_hex)reg=Number.parseInt(c.write_hex,16);
    const bytes=c.read_len?(reg===0?'88 57':'88 00'):'—';
    const write=c.write_hex?c.write_hex.match(/../g).join(' ').toUpperCase():'—';
    return {data:{detail:{response:`I2C 0x2B @100000Hz write ${write} read ${bytes}`}}};
  }});
  const rx=createMT5788(bus);assert.equal((await rx.identify()).variant,'HW');
  assert.deepEqual(commands.map(c=>[c.write_hex,c.read_len]),[['0000',0],['',2],['0002',0],['',2]]);
  lease=false;await assert.rejects(rx.readRxTelemetry(),/LEASE.LOST/);assert.equal(commands.length,4);
});

test('FT4222 rejects incomplete and mismatched results',async()=>{
  for(const response of ['I2C 0x2B @100000Hz write — read 88','I2C 0x2C @100000Hz write — read 88 57','nonsense',null]){
    const bus=createFT4222Bus({checkLease:async()=>{},executeConfirmed:async()=>({data:{detail:{response}}})});
    await assert.rejects(bus.read(0x2b,2),/I2C\.(RESPONSE|SHORT)/);
  }
});
