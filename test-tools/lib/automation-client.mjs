// Generic SW client. Product rules, fixture control and Dongle binary ICD belong elsewhere.
import {readFile} from 'node:fs/promises';
import path from 'node:path';
export class AutomationClient {
  constructor(root) { this.root=path.resolve(root); }
  async request(route, payload, timeoutMs=5000) {
    const config=JSON.parse(await readFile(path.join(this.root,'service-token.json'),'utf8'));
    if (!Number.isInteger(config.port)||config.port<1||config.port>65535||typeof config.token!=='string') throw new Error('AUTOMATION.INVALID_SERVICE_CONFIG');
    const r=await fetch(`http://127.0.0.1:${config.port}${route}`,{
      method:payload===undefined?'GET':'POST',
      headers:{Authorization:`Bearer ${config.token}`,'Content-Type':'application/json'},
      body:payload===undefined?undefined:JSON.stringify(payload),signal:AbortSignal.timeout(timeoutMs)
    });
    if(!r.ok)throw new Error(`AUTOMATION.HTTP_${r.status}`);
    const v=await r.json();
    if(!v.ok)throw Object.assign(new Error(v.error),{code:v.error,error_record:v.error_record??null});
    return v.data;
  }
  invoke(method,params={}){return this.request('/v1/invoke',{method,params});}
  async wait(jobId,{timeoutMs=120000,intervalMs=100,signal}={}){
    if(!Number.isFinite(timeoutMs)||timeoutMs<=0||!Number.isFinite(intervalMs)||intervalMs<=0)throw new Error('JOB.INVALID_WAIT_BUDGET');
    const end=performance.now()+timeoutMs;
    while(performance.now()<end){
      signal?.throwIfAborted();const job=await this.request('/v1/invoke',{method:'job.get',params:{job_id:jobId}},Math.max(1,Math.floor(Math.min(1000,end-performance.now()))));
      signal?.throwIfAborted();
      if(performance.now()>=end)throw new Error('JOB.CLIENT_WAIT_TIMEOUT');
      if(['succeeded','failed','cancelled','unknown'].includes(job.state))return job;
      await new Promise(r=>setTimeout(r,Math.min(intervalMs,Math.max(0,end-performance.now()))));
    }
    // Never cancel/re-submit automatically: caller must query and decide recovery.
    throw new Error('JOB.CLIENT_WAIT_TIMEOUT');
  }
}
