import {Worker} from 'node:worker_threads';
import {fail} from './common.mjs';
export async function evaluateBounded(rules,evidence,{timeoutMs=2000,stopped=()=>false}={}) {
  if(stopped())throw fail('RUN.ABORTED');
  if(timeoutMs<=0)throw fail('RULE.TIMEOUT');
  const worker=new Worker(new URL('./rule-worker.mjs',import.meta.url),{workerData:{rules,evidence},execArgv:[]});
  let timeout,poll;
  try {return await new Promise((resolve,reject)=>{
    timeout=setTimeout(()=>reject(fail('RULE.TIMEOUT')),timeoutMs);
    poll=setInterval(()=>{if(stopped())reject(fail('RUN.ABORTED'));},20);
    worker.once('message',reply=>reply.ok?resolve(reply.value):reject(fail('RULE.ERROR',reply.error)));
    worker.once('error',reject);worker.once('exit',code=>{if(code!==0)reject(fail('RULE.WORKER_EXIT',String(code)));});
  });}finally{clearTimeout(timeout);clearInterval(poll);await worker.terminate();}
}
