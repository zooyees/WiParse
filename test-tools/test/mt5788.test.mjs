import test from 'node:test';
import assert from 'node:assert/strict';
import {createMT5788} from '../plugins/wireless-whitebox/mt5788.mjs';

function mock(customer=0x88) {
  const writes=[];let reg=0;
  const memory={0:[0x88,0x57],2:[customer,0],0x18:[2,0,0,0],0x52:[0x34,0x12],0x54:[1,0],0x56:[2,0],0x58:[3,0],0x5a:[4,0]};
  return {writes,async write(address,bytes){assert.equal(address,0x2b);writes.push(bytes);reg=bytes[0]*256+bytes[1];},async read(address,n){assert.equal(address,0x2b);return (memory[reg]||[]).slice(0,n);}};
}
for(const [customer,variant,register,command] of [[0x88,'HW',0xbe,[0,8,0,0]],[0x0a,'HH',0xbc,[0,4,0,0]]]) {
  test(`MT5788 ${variant} identity, telemetry and limit-guarded VOUT sequence`,async()=>{
    const bus=mock(customer), waits=[];const chip=createMT5788(bus,{sleep:async ms=>waits.push(ms)});
    await assert.rejects(chip.readRxTelemetry(),/IDENTITY_REQUIRED/);
    assert.equal((await chip.identify()).variant,variant);
    assert.equal((await chip.readRxTelemetry()).values_raw.fop,0x1234);
    await assert.rejects(chip.setVoutMv(5500,{}),/LIMIT_REQUIRED/);
    await chip.setVoutMv(5500,{minimum_mv:5000,maximum_mv:6000});
    assert.deepEqual(bus.writes.slice(-3),[[0,0x18],[0,register,0x7c,0x15],[0,0x2c,...command]]);
    assert.deepEqual(waits,[100]);
  });
}
test('MT5788 refuses unknown customer, short reads and uncertain writes',async()=>{
  const unknown=createMT5788(mock(0x42));await assert.rejects(unknown.identify(),/CUSTOMER_UNSUPPORTED/);
  const short=createMT5788({write:async()=>{},read:async()=>[]});await assert.rejects(short.identify(),/SHORT_READ/);
  const bus=mock(), chip=createMT5788(bus,{sleep:async()=>{throw new Error('interrupted');}});await chip.identify();
  await assert.rejects(chip.setVoutMv(5000,{minimum_mv:4000,maximum_mv:6000}),/OUTPUT_STATE_UNKNOWN/);
  assert.equal(bus.writes.some(w=>w[1]===0x2c),false);
});

test('concurrent telemetry operations do not interleave register address and read',async()=>{
  const bus=mock(), originalRead=bus.read.bind(bus),events=[];
  const wrapped={async write(addr,bytes){events.push(`w:${bytes[1]}`);return bus.write(addr,bytes);},async read(addr,n){events.push(`r:${n}`);await new Promise(resolve=>setTimeout(resolve,1));return originalRead(addr,n);}};
  const chip=createMT5788(wrapped);await chip.identify();events.length=0;
  await Promise.all([chip.readRxTelemetry(),chip.readRxTelemetry()]);
  assert.equal(events.length,24);
  for(let i=0;i<events.length;i+=2){assert.match(events[i],/^w:/);assert.match(events[i+1],/^r:/);}
});
