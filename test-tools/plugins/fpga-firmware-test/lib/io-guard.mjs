import {performance} from 'node:perf_hooks';
import {randomUUID} from 'node:crypto';
import {fail} from './common.mjs';
export const CANCEL_PENDING=Symbol('fpga.cancel-pending');
export function guardedAdapter(adapter,{stopped,deadline,clock,onUnknown=()=>{},cleanupTimeoutMs=10000}){
  let unknown=false;
  const active=new Set();
  const methods=new Set(['sourceOutput','voltage','probeReset','collectLog','capture','waveformSource','measure','cleanup']);
  const request=(operation,reason)=>{
    if(operation.reason)return;
    operation.reason=reason;operation.controller.abort();operation.wake({cancel:reason});
  };
  const requestAll=reason=>{for(const operation of active)request(operation,reason);};
  const cancelPending=async reason=>{
    const operations=[...active];requestAll(reason);
    await Promise.all(operations.map(operation=>operation.done));
  };
  return new Proxy(adapter,{get(target,key){
    if(key===CANCEL_PENDING)return cancelPending;
    if(!methods.has(key)||typeof target[key]!=='function')return typeof target[key]==='function'?target[key].bind(target):target[key];
    return async(...args)=>{
      if(unknown)throw fail('IO.STATE_UNKNOWN','Transport is awaiting recovery');
      if([...active].some(operation=>operation.reason))throw fail('IO.CANCELLING','Wait for cancellation confirmation');
      const cleanup=key==='cleanup',operation_id=randomUUID(),controller=new AbortController();
      const remaining=cleanup?cleanupTimeoutMs:Math.max(0,deadline-clock.now());
      if(!cleanup&&stopped())throw fail('RUN.ABORTED');
      if(!remaining)throw fail('IO.TIMEOUT',String(key));
      const configured=typeof args[0]==='object'?args[0]?.config?.timeout_s:undefined;
      const budget=Math.min(remaining,configured?configured*1000:remaining);
      const control={operation_id,signal:controller.signal,deadline_mono_ms:clock.now()+budget};
      if(args.length===1&&args[0]&&typeof args[0]==='object')args[0]={...args[0],...control};else args.push(control);
      let done,wake,poll,timer;
      const operation={controller,done:new Promise(resolve=>{done=resolve;}),wake:null,reason:null};
      const cancellation=new Promise(resolve=>{wake=resolve;});operation.wake=wake;active.add(operation);
      const confirm=async reason=>{
        let confirmation;
        try{confirmation=await Promise.race([
          Promise.resolve().then(()=>target.cancelAndConfirm?.({operation_id,method:key,reason})),
          new Promise(resolve=>{timer=setTimeout(()=>resolve(null),1000);})
        ]);}catch{confirmation=null;}
        if(confirmation?.quiescent!==true){
          if(!unknown){unknown=true;onUnknown();}
          requestAll('IO.STATE_UNKNOWN');
          throw fail('IO.STATE_UNKNOWN',`${key}: ${reason}; manual recovery required`);
        }
      };
      try{
        const started=performance.now();
        poll=setInterval(()=>{if((!cleanup&&stopped())||performance.now()-started>=budget)requestAll(!cleanup&&stopped()?'RUN.ABORTED':'IO.TIMEOUT');},Math.min(20,Math.max(1,budget)));
        const outcome=await Promise.race([
          Promise.resolve().then(()=>target[key](...args)).then(value=>({value}),error=>({error,rejected:true})),cancellation
        ]);
        // Only explicitly quiescent business errors bypass cancellation confirmation.
        if(outcome.rejected&&!operation.reason){
          if(outcome.error?.quiescent===true)throw outcome.error;
          requestAll('IO.TRANSPORT');
        }
        if(operation.reason){
          await confirm(operation.reason);
          const error=fail(operation.reason,String(key));
          if(outcome.rejected){error.cause=outcome.error;error.message+=`: ${outcome.error?.message??outcome.error}`;}
          throw error;
        }
        return outcome.value;
      }finally{clearInterval(poll);clearTimeout(timer);active.delete(operation);done();}
    };
  }});
}
