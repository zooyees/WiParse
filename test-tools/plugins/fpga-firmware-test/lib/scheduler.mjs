import { performance } from 'node:perf_hooks';
import { fail } from './common.mjs';
import { preserveLog } from './log-capture.mjs';
import fs from 'node:fs';
import path from 'node:path';
import { hash } from './common.mjs';
import {CANCEL_PENDING} from './io-guard.mjs';
function artifact(value, dir) {
  if(!value?.path) throw fail('EVIDENCE.MISSING_ARTIFACT');
  const absolute=fs.realpathSync(path.resolve(dir,value.path)), base=fs.realpathSync(dir), relative=path.relative(base,absolute);
  if(relative.startsWith('..')||path.isAbsolute(relative)) throw fail('EVIDENCE.PATH_ESCAPE');
  const sha256=hash(fs.readFileSync(absolute));
  if(value.sha256!==sha256) throw fail('EVIDENCE.HASH_MISMATCH');
  return {...value,path:relative.split(path.sep).join('/'),sha256};
}
export const clock = {now:()=>performance.now(),sleep:ms=>new Promise(r=>setTimeout(r,ms))};
export async function waitUntil(time, clock, stopped) {
  while(clock.now()<time) {if(stopped()) throw fail('RUN.ABORTED'); await clock.sleep(Math.max(0,Math.min(25,time-clock.now())));}
  if(stopped()) throw fail('RUN.ABORTED');
}
export async function resetDut(c, resources, adapter, clock, stopped) {
  const started=clock.now(); let confirmation;
  if(c.method==='dc_power') {
    await adapter.sourceOutput(resources[c.power_role],c.channel,false);
    await waitUntil(clock.now()+c.off_time_ms,clock,stopped);
    await adapter.sourceOutput(resources[c.power_role],c.channel,true);
    const ack=clock.now(), spec=c.release_confirmation;
    if(spec.type==='voltage_threshold') {
      const deadline=ack+spec.timeout_ms; let stable=null;
      while(clock.now()<=deadline) {
        if(stopped()) throw fail('RUN.ABORTED');
        const voltage=await adapter.voltage(resources[c.power_role],c.channel);
        if(voltage>=spec.minimum_v) {stable??=clock.now(); if(clock.now()-stable>=spec.stable_ms) break;} else stable=null;
        await clock.sleep(5);
      }
      if(stable===null || clock.now()>deadline) throw fail('RESET.RELEASE_TIMEOUT');
    }
    confirmation=spec.type;
  } else {await adapter.probeReset(resources[c.dongle_role],{hardware:true}); confirmation='command_complete';}
  return {method:c.method,started_mono_ms:started,capture_t0:clock.now(),utc:new Date().toISOString(),confirmation,reset_duration_ms:clock.now()-started,timing_uncertainty_ms:null};
}
export async function capture(c, resources, adapter, clock, stopped, event, dir, station) {
  const parentStopped=stopped;let cancelled=false;
  stopped=()=>cancelled||parentStopped();
  const t0=event.capture_t0, evidence={metrics:{},artifacts:{}};
  const jobs=[];
  if(c.log_capture.enabled) jobs.push((async()=>{
    await waitUntil(t0+c.log_capture.start_at_s*1000,clock,stopped);
    evidence.log=preserveLog(await adapter.collectLog({item_id:c.id,resource:resources[c.log_capture.resource_role??'log'],config:c.log_capture,start_mono_ms:t0+c.log_capture.start_at_s*1000,end_mono_ms:t0+(c.log_capture.start_at_s+c.log_capture.duration_s)*1000,dir,stopped}),dir,c.log_capture);
  })());
  jobs.push((async()=>{
    for(const w of [...c.waveform_captures].sort((a,b)=>a.at_s-b.at_s)) {
      await waitUntil(t0+w.at_s*1000,clock,stopped);
      const actual=clock.now(), lateness=actual-(t0+w.at_s*1000);
      if(lateness>(station.late_tolerance_ms??100)) throw fail('WAVEFORM.LATE',w.id);
      const result=await adapter.capture({item_id:c.id,resource:resources.scope,config:w,dir,stopped});
      evidence.artifacts[w.id]=artifact(result.screenshot,dir);
      if(stopped()) throw fail('RUN.ABORTED');
      if(w.source_waveform?.enabled) {
        const source=await adapter.waveformSource({item_id:c.id,resource:resources.scope,config:w,dir,stopped});
        evidence.metrics[w.id]=source.metrics; evidence.artifacts[`${w.id}.source`]=artifact(source.artifact,dir);
      }
      (evidence.capture_events??=[]).push({id:w.id,planned_mono_ms:t0+w.at_s*1000,actual_mono_ms:actual,completed_mono_ms:clock.now(),lateness_ms:lateness});
    }
  })());
  const settled=await Promise.allSettled(jobs.map(job=>job.catch(async error=>{
    cancelled=true;
    await adapter[CANCEL_PENDING]?.('IO.SIBLING_FAILED');
    throw error;
  })));
  const error=settled.find(r=>r.status==='rejected'&&String(r.reason?.code).startsWith('IO.'))??settled.find(r=>r.status==='rejected');
  if(error) throw Object.assign(error.reason,{evidence});
  return evidence;
}
