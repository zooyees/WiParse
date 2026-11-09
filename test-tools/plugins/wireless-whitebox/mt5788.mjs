// MT5788 runtime register interface based on MaxicCTP MT5788Base.py and
// dongle_manager.py. Transport must serialize these separate write/read calls.
// Hardware usage requires an independently verified owner and safe limits.
export const MT5788_ADDRESS=0x2b;
const REG={chip:0x0000,customer:0x0002,sys:0x0018,cmd:0x002c,fop:0x0052,iout:0x0054,vout:0x0056,vrect:0x0058,temp:0x005a};
const VARIANTS=Object.freeze({136:{name:'HW',vout:0x00be,command:1<<11},10:{name:'HH',vout:0x00bc,command:1<<10}});
const regBytes=reg=>[reg>>>8,reg&255];
function unsigned(value,max,label) {if(!Number.isInteger(value)||value<0||value>max)throw new Error(`MT5788.INVALID_${label}`);return value;}
function payload(bytes) {
  if(!Array.isArray(bytes)&&!(bytes instanceof Uint8Array)) throw new Error('MT5788.INVALID_READ');
  return Array.from(bytes);
}
export function createMT5788(bus,{address=MT5788_ADDRESS,sleep=ms=>new Promise(resolve=>setTimeout(resolve,ms))}={}) {
  unsigned(address,0x7f,'ADDRESS');
  if(!bus || typeof bus.write!=='function'||typeof bus.read!=='function')throw new Error('MT5788.BUS_REQUIRED');
  let variant=null,verified=false;
  async function readReg(reg,n) {
    await bus.write(address,regBytes(reg));
    const data=payload(await bus.read(address,n));
    if(data.length!==n||data.some(v=>!Number.isInteger(v)||v<0||v>255))throw new Error('MT5788.SHORT_READ');
    return data.reduce((value,byte,i)=>value+byte*2**(8*i),0);
  }
  async function writeReg(reg,value,n) {
    unsigned(value,n===2?0xffff:0xffffffff,'VALUE');
    await bus.write(address,[...regBytes(reg),...Array.from({length:n},(_,i)=>Math.floor(value/2**(8*i))&255)]);
  }
  const device = {
    async identify() {
      verified=false;variant=null;
      const chip=await readReg(REG.chip,2);
      if(chip!==0x5788)throw new Error('MT5788.CHIP_ID_MISMATCH');
      const customer=await readReg(REG.customer,2);
      if(!VARIANTS[customer])throw new Error('MT5788.CUSTOMER_UNSUPPORTED');
      variant=VARIANTS[customer];verified=true;
      return {chip,customer,variant:variant.name,address};
    },
    async readRxTelemetry() {
      if(!verified)throw new Error('MT5788.IDENTITY_REQUIRED');
      const sys=await readReg(REG.sys,4);
      if(!(sys & (1<<1)))throw new Error('MT5788.NOT_RX_MODE');
      const fields={};
      for(const key of ['fop','iout','vout','vrect','temp'])fields[key]=await readReg(REG[key],2);
      return {sys_mode_raw:sys,values_raw:fields,units:'unverified',source:'MT5788 registers',variant:variant.name};
    },
    async setVoutMv(value,{minimum_mv,maximum_mv}={}) {
      if(!verified)throw new Error('MT5788.IDENTITY_REQUIRED');
      unsigned(value,0xffff,'VOUT');
      if(!Number.isInteger(minimum_mv)||!Number.isInteger(maximum_mv)||minimum_mv<0||maximum_mv>0xffff||minimum_mv>maximum_mv||value<minimum_mv||value>maximum_mv)throw new Error('MT5788.VOUT_LIMIT_REQUIRED');
      const sys=await readReg(REG.sys,4);
      if(!(sys & (1<<1)))throw new Error('MT5788.NOT_RX_MODE');
      // On failure the output state may have changed; never retry automatically.
      try {await writeReg(variant.vout,value,2);await sleep(100);await writeReg(REG.cmd,variant.command,4);}
      catch(error){const e=new Error('MT5788.OUTPUT_STATE_UNKNOWN');e.cause=error;throw e;}
      return {requested_mv:value,variant:variant.name,physical_output_verified:false};
    },
  };
  // Keep register-address writes and their following reads together even if
  // multiple callers ask for telemetry or controls concurrently.
  let tail=Promise.resolve();
  return Object.fromEntries(Object.entries(device).map(([key,fn])=>[key,(...args)=>{
    const job=tail.then(()=>fn(...args));
    tail=job.catch(()=>{});
    return job;
  }]));
}
