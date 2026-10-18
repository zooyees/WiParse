import {performance} from 'node:perf_hooks';
import {setTimeout as delay} from 'node:timers/promises';
import {fail} from './common.mjs';

// Only retry a mutex acquisition which has not entered the operation. Never
// rerun a plugin lifecycle, a lease operation, recovery, or a committed mutation.
export async function retryWorkspaceLock(operation,{stopped=()=>false,timeoutMs=2000,intervalMs=25,now=()=>performance.now(),sleep=delay,onWait=()=>{}}={}) {
  const deadline=now()+timeoutMs;let last,notified=false;
  for(;;) {
    if(stopped())fail('RUN.ABORTED','Stopped while waiting for the configuration repository');
    if(last && now()>=deadline)throw last;
    try {return await operation();}
    catch(error) {
      if(error.code!=='CONFIG.BUSY'||error.data?.lock_contention!==true)throw error;
      last=error;
      const remaining=deadline-now();if(remaining<=0)throw error;
      if(!notified){notified=true;onWait();}
      await sleep(Math.min(intervalMs,remaining));
    }
  }
}
