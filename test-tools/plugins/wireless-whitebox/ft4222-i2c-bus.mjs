// The caller must hold WiParse's device lease and wait for the matching GUI job.
// One BridgeI2c call with only a write or only a read preserves the transaction
// sequence used by the MaxicCTP MT5788 implementation.
export function createFT4222Bus({executeConfirmed,checkLease,clock_hz=100000}) {
  if(typeof executeConfirmed!=='function'||typeof checkLease!=='function')throw new Error('I2C.OWNER_REQUIRED');
  if(!Number.isInteger(clock_hz)||clock_hz<1||clock_hz>1000000)throw new Error('I2C.CLOCK_INVALID');
  async function transfer(address,write,read_len) {
    if(!Number.isInteger(address)||address<0||address>127)throw new Error('I2C.ADDRESS_INVALID');
    if(!Array.isArray(write)||write.length>256||write.some(v=>!Number.isInteger(v)||v<0||v>255))throw new Error('I2C.DATA_INVALID');
    if(!Number.isInteger(read_len)||read_len<0||read_len>256||(!write.length&&!read_len))throw new Error('I2C.LENGTH_INVALID');
    await checkLease();
    const addr=address.toString(16).padStart(2,'0').toUpperCase();
    const write_hex=Buffer.from(write).toString('hex');
    const result=await executeConfirmed({BridgeI2c:{addr,write_hex,read_len,clock_hz}});
    const response=result?.data?.detail?.response ?? result?.detail?.response;
    if(typeof response!=='string')throw new Error('I2C.RESPONSE_MISSING');
    const m=response.match(/^I2C 0x([0-9a-f]{2}) @(\d+)Hz write (.*?) read (.*?)\s*$/i);
    if(!m||Number.parseInt(m[1],16)!==address||Number(m[2])!==clock_hz)throw new Error('I2C.RESPONSE_INVALID');
    const written=m[3]==='—'?'':m[3].replace(/\s+/g,'').toLowerCase();
    if(written!==write_hex)throw new Error('I2C.RESPONSE_INVALID');
    const data=m[4]==='—'?[]:m[4].trim().split(/\s+/);
    if(data.length!==read_len||data.some(v=>!/^[0-9a-f]{2}$/i.test(v)))throw new Error('I2C.SHORT_READ');
    return data.map(v=>Number.parseInt(v,16));
  }
  return {write:(address,bytes)=>transfer(address,bytes,0),read:(address,n)=>transfer(address,[],n)};
}
