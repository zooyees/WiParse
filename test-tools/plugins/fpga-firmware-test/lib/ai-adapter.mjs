import { digest } from './common.mjs';
export function validateAI(config, station) {
  if(!config?.enabled) return;
  const p=station.ai_profiles?.[config.provider_profile];
  if(!p || !['local','public'].includes(p.kind) || !p.model || !p.adapter || p.api_key || p.token) throw new Error('AI.PROFILE_INVALID');
  if(config.policy!=='advisory_only') throw new Error('AI.POLICY_INVALID');
  if(p.kind==='public' && (!p.credential_ref || !p.allow_fields || !config.redaction_profile)) throw new Error('AI.PUBLIC_POLICY_MISSING');
}
export async function advise(config, station, adapter, evidence, rules) {
  if(!config?.enabled) return {status:'disabled'};
  const p=station.ai_profiles[config.provider_profile];
  if(p.kind==='public' && !station.allow_public_ai) return {status:'skipped',reason:'external_transfer_not_authorized'};
  // No raw log, DUT identity, firmware or raw waveform enters the provider boundary.
  const allowed={rule_results:rules.results.map(({id,verdict,measured,unit})=>({id,verdict,measured,unit})),waveform_metrics:evidence.metrics??{}};
  const inputs=config.inputs??['rule_results','waveform_metrics'];
  const allowlist=p.allow_fields??(p.kind==='local'?Object.keys(allowed):[]);
  const payload=Object.fromEntries(Object.entries(allowed).filter(([k])=>inputs.includes(k)&&allowlist.includes(k)));
  const audit={policy:'advisory_only',model:p.model,prompt_template_id:config.prompt_template_id??'fpga-fault-triage-v1',sent_fields:Object.keys(payload),request_sha256:digest(payload),redacted_input:payload};
  let timer;
  try {
    const response=await Promise.race([adapter.ai({profile:config.provider_profile,payload,audit}),new Promise((_,reject)=>{timer=setTimeout(()=>reject(new Error('AI.TIMEOUT')),(config.timeout_s??30)*1000);})]);
    if(!['PASS','FAIL','REVIEW'].includes(response.recommendation)||!Number.isFinite(response.confidence)||response.confidence<0||response.confidence>1) throw new Error('AI.INVALID_RESPONSE');
    return {...audit,status:'completed',response};
  } catch(e) { return {...audit,status:'error',error:e.message}; } finally {clearTimeout(timer);}
}
