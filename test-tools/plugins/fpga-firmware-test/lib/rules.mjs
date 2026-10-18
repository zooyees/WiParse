import { digest } from './common.mjs';
import { ruleErrors } from './rule-contract.mjs';
export function evaluate(rules, evidence) {
  const one = r => {
    let verdict='INCONCLUSIVE', measured=null, children;
    if(ruleErrors(r).length) return {id:r?.id,type:r?.type,required:r?.required!==false,verdict,measured,reason:'invalid_rule',errors:ruleErrors(r)};
    const validLog=evidence.log?.complete && evidence.log?.decoded && !evidence.log?.truncated;
    if(r.rules) {
      children=r.rules.map(one); const vs=children.map(x=>x.verdict);
      if(r.type==='all') verdict=vs.includes('FAIL')?'FAIL':vs.every(v=>v==='PASS')?'PASS':'INCONCLUSIVE';
      if(r.type==='any') verdict=vs.includes('PASS')?'PASS':vs.every(v=>v==='FAIL')?'FAIL':'INCONCLUSIVE';
      if(r.type==='not' && vs.length===1) verdict=vs[0]==='PASS'?'FAIL':vs[0]==='FAIL'?'PASS':'INCONCLUSIVE';
    } else if(r.type.startsWith('log_') && validLog) {
      const text=evidence.log.text;
      measured=r.type==='log_contains' ? Number(text.includes(r.text)) : [...text.matchAll(new RegExp(r.pattern,'g'))].length;
      verdict=(r.type==='log_not_regex' ? measured===0 : measured>=(r.minimum_matches??1) && measured<=(r.maximum_matches??Infinity))?'PASS':'FAIL';
    } else if(r.type.endsWith('_range')) {
      const m=r.type==='waveform_metric_range' ? evidence.metrics?.[r.capture]?.[r.channel]?.[r.metric] : evidence.measurements?.[r.id];
      if(m?.quality==='good' && m.unit===r.unit && Number.isFinite(m.value)) { measured=m.value; verdict=measured>=r.minimum && measured<=r.maximum?'PASS':'FAIL'; }
    } else if(['file_exists','artifact_hash'].includes(r.type)) {
      const a=evidence.artifacts?.[r.evidence_ref];
      if(a) { measured=a.sha256; verdict=r.type==='file_exists'||a.sha256===r.sha256?'PASS':'FAIL'; }
    }
    return {id:r.id,type:r.type,required:r.required!==false,verdict,measured,minimum:r.minimum,maximum:r.maximum,unit:r.unit,source:r.limit_source,limit_sha256:r.limit_sha256,evidence_refs:r.evidence_refs??[r.evidence_ref??r.capture??'logs/serial.jsonl'],reason:verdict==='PASS'?'satisfied':verdict==='FAIL'?'rule_not_satisfied':'missing_or_invalid_evidence',children};
  };
  const results=rules.map(one), required=results.filter(r=>r.required);
  return {results,product_verdict:required.some(r=>r.verdict==='FAIL')?'FAIL':required.length && required.every(r=>r.verdict==='PASS')?'PASS':'INCONCLUSIVE',limits_sha256:digest(rules)};
}
