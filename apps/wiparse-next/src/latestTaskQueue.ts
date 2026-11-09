type Clock={now:()=>number;set:(callback:()=>void,delay:number)=>number;clear:(id:number)=>void};
/** One running job and one replaceable pending job. Updates do not restart the
 * throttle, so a continuous gesture makes progress instead of waiting for idle.
 * Completed jobs may advance the same source while a newer target is pending;
 * invalidate prevents any completion from a previous source being published. */
export function latestTaskQueue<Input,Output>(run:(input:Input,alive:()=>boolean)=>Promise<Output>,accept:(output:Output,input:Input)=>void,reject:(reason:unknown,input:Input)=>void,interval=40,clock:Clock={now:()=>performance.now(),set:(callback,delay)=>globalThis.setTimeout(callback,delay),clear:id=>clearTimeout(id)}){
 if(!Number.isFinite(interval)||interval<0)throw Error('Invalid task interval');
 let epoch=0,active=false,disposed=false,timer:number|null=null,lastStart:number|null=null;
 let pending:{input:Input;epoch:number}|null=null;
 const schedule=()=>{
  if(disposed||active||timer!==null||!pending)return;
  const delay=lastStart===null?0:Math.max(0,lastStart+interval-clock.now());
  if(delay===0)start();else timer=clock.set(()=>{timer=null;start();},delay);
 };
 const start=()=>{
  if(disposed||active||!pending)return;
  const job=pending;pending=null;active=true;lastStart=clock.now();
  const alive=()=>!disposed&&job.epoch===epoch;
  void Promise.resolve().then(()=>{if(!alive())throw Error('Task invalidated');return run(job.input,alive);}).then(output=>{
   if(!disposed&&job.epoch===epoch)accept(output,job.input);
  }).catch(reason=>{
   // A failure from a superseded target must not cover a newer pending view.
   if(!disposed&&job.epoch===epoch&&!pending)reject(reason,job.input);
  }).finally(()=>{active=false;schedule();});
 };
 const invalidate=()=>{epoch++;pending=null;lastStart=null;if(timer!==null)clock.clear(timer);timer=null;};
 return {submit(input:Input){if(disposed)return;pending={input,epoch};schedule();},invalidate,dispose(){disposed=true;invalidate();}};
}
