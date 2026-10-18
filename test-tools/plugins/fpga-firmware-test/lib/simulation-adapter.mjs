/** Explicit simulation transport; never substituted for a missing hardware adapter. */
import fs from 'node:fs';
import path from 'node:path';
import {REQUIRED_CAPABILITIES} from './host-adapter.mjs';
import {hash,fail} from './common.mjs';
import {DEMO_FIXTURES} from './demo-fixtures.mjs';
export function simulationAdapter(ctx){
  if(ctx.args?.execution_mode!=='simulation')return null;
  const devices=()=>Object.values(ctx.projectConfig?.value?.instruments??{}).map((role,index)=>({kind:role.kind,...role.selector,device_id:index+1,model:role.selector?.model||'SIMULATED',simulation_only:true}));
  const unique=()=>[...new Map(devices().map(d=>[JSON.stringify([d.kind,d.serial,d.resource,d.port,d.dongle_serial,d.uart_channel]),d])).values()];
  let cursor=0;
  const artifact=(file,bytes)=>{fs.mkdirSync(path.dirname(file),{recursive:true});fs.writeFileSync(file,bytes);return {path:file,sha256:hash(bytes)};};
  return {version:'wiparse.fpga-host/1',capabilities:REQUIRED_CAPABILITIES,simulation_only:true,
    devices:async()=>unique(),preflight:async()=>{},storage:async()=>{},acquire:async()=>({simulation_only:true}),release:async()=>{},cancelAndConfirm:async()=>{await new Promise(r=>setTimeout(r,25));return {quiescent:true};},
    sourceOutput:async()=>{},voltage:async()=>12,probeReset:async()=>{},cleanup:async()=>({ok:true,simulation_only:true}),
    collectLog:async({item_id,start_mono_ms,end_mono_ms,stopped,signal})=>{
      while(performance.now()<end_mono_ms){if(stopped()||signal?.aborted)throw fail('RUN.ABORTED');await new Promise(r=>setTimeout(r,10));}
      const bad=ctx.args.simulation_scenario==='fail',text=bad?'SIMULATION: BOOT_ERROR FATAL\n':`SIMULATION: ${DEMO_FIXTURES[item_id]?.token??'BOOT_OK'}\n`;
      const start=cursor;cursor+=Buffer.byteLength(text);
      return {raw_base64:Buffer.from(text).toString('base64'),lines:[{mono_ms:start_mono_ms,utc:new Date().toISOString(),text:text.trim()}],start_cursor:start,end_cursor:cursor,dropped_lines:0,window_covered:true,complete:true,simulation_only:true};
    },
    capture:async({dir,config})=>{
      // Static labelled diagnostic image, never presented as an instrument screenshot.
      const bytes=fs.readFileSync(new URL('../assets/simulation-waveform.png',import.meta.url));
      return {screenshot:artifact(path.join(dir,'shots',config.id+'-SIMULATION.png'),bytes),simulation_only:true};
    },
    waveformSource:async({dir,config,item_id})=>{
      const frequency=DEMO_FIXTURES[item_id]?.frequency??100000;
      const bytes=Buffer.from('time_s,CH1_V\n'+Array.from({length:2000},(_,i)=>`${i/(frequency*100)},${Math.floor(i/50)%2?0:5}`).join('\n')+'\n');
      return {artifact:artifact(path.join(dir,'waves',config.id+'-SIMULATION.csv'),bytes),metrics:Object.fromEntries(config.channels.map(ch=>[ch,{frequency_hz:{value:frequency,unit:'Hz',quality:'good'},duty_cycle:{value:0.5,unit:'ratio',quality:'good'}}])),simulation_only:true};
    },
    measure:async()=>({value:12,unit:'V',quality:'good',simulation_only:true})};
}
