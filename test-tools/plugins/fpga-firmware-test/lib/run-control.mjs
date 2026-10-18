import fs from 'node:fs';
import {atomic,fail} from './common.mjs';
export async function pauseBoundary({file,runId,clock,stopped,changed}) {
  const start=clock.now();let paused=false;
  for(;;){
    if(stopped())throw fail('RUN.ABORTED');
    const control=fs.existsSync(file)?JSON.parse(fs.readFileSync(file,'utf8')):{};
    if(control.run_id!==runId||control.action!=='pause'){
      if(control.run_id===runId&&control.state==='resume_requested'){atomic(file,{run_id:runId,state:'running',action:'resume',updated_at:new Date().toISOString()});changed('running');}
      return paused?clock.now()-start:0;
    }
    if(control.state!=='paused'){atomic(file,{...control,state:'paused',updated_at:new Date().toISOString()});changed('paused');}
    paused=true;await clock.sleep(50);
  }
}
