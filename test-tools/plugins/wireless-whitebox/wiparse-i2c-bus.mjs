import {parseI2cReply} from '../../lib/wiparse-if.mjs';

// executeConfirmed must resolve only after the matching GUI instrument job
// completes. checkLease must reject when this caller no longer owns the Dongle.
// This module neither opens hardware nor grants a lease itself.
export function createWiParseI2cBus({executeConfirmed,checkLease}) {
  if(typeof executeConfirmed!=='function'||typeof checkLease!=='function')throw new Error('I2C.OWNER_REQUIRED');
  async function execute(kind,address,bytesOrLength) {
    if(!Number.isInteger(address)||address<0||address>127)throw new Error('I2C.ADDRESS_INVALID');
    await checkLease();
    const addr=address.toString(16).padStart(2,'0').toUpperCase();
    let command;
    if(kind==='write') {
      if(!Array.isArray(bytesOrLength)||!bytesOrLength.length||bytesOrLength.length>256||bytesOrLength.some(v=>!Number.isInteger(v)||v<0||v>255))throw new Error('I2C.DATA_INVALID');
      command={DongleI2cWrite:{addr,data_hex:Buffer.from(bytesOrLength).toString('hex')}};
    } else {
      if(!Number.isInteger(bytesOrLength)||bytesOrLength<1||bytesOrLength>256)throw new Error('I2C.LENGTH_INVALID');
      command={DongleI2cRead:{addr,len:bytesOrLength}};
    }
    const result=await executeConfirmed(command);
    const response=result?.data?.detail?.response ?? result?.detail?.response;
    if(typeof response!=='string')throw new Error('I2C.RESPONSE_MISSING');
    const parsed=parseI2cReply(response);
    if(!parsed.ok || parsed.tag!==(kind==='write'?'WRITE':'READ'))throw new Error('I2C.RESPONSE_ERROR');
    if(kind==='read' && parsed.hex.length!==bytesOrLength)throw new Error('I2C.SHORT_READ');
    return kind==='read'?[...parsed.hex]:undefined;
  }
  return {write:(address,bytes)=>execute('write',address,bytes),read:(address,n)=>execute('read',address,n)};
}
