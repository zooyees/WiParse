export function aggregate(checks, execution='completed') {
  if(checks.some(c=>c.product_verdict==='FAIL')) return 'FAIL';
  if(execution!=='completed'||!checks.length||checks.some(c=>!['PASS','SKIP'].includes(c.product_verdict))) return 'INCONCLUSIVE';
  if(checks.some(c=>c.product_verdict==='SKIP' && !c.skip_approval)) return 'INCONCLUSIVE';
  return checks.some(c=>c.product_verdict==='PASS')?'PASS':'INCONCLUSIVE';
}
const verdict=(v,r,refs=[])=>({product_verdict:v,reason_code:r,evidence_refs:refs});
export function stateRule(check, observation, states) {
  if(!observation?.events?.length) return verdict('INCONCLUSIVE','STATE.NO_EVIDENCE');
  const events=observation.events, refs=events.flatMap(e=>e.evidence_refs||[]);
  if(check.expected_initial && events[0][check.layer]!==check.expected_initial) return verdict('INCONCLUSIVE','STATE.INITIAL_NOT_OBSERVED',refs);
  let previous=null, time=-Infinity;
  for(const event of events) {
    if(!Number.isFinite(event.time_s)||event.time_s<time||!event.evidence_refs?.length||!Array.isArray(event.quality?.issues)||event.quality.issues.length) return verdict('INCONCLUSIVE','STATE.QUALITY',refs);
    const layer=check.layer;
    const current=event[layer];
    if(!states.layers[layer]?.includes(current)) return verdict('INCONCLUSIVE','STATE.UNKNOWN',refs);
    if(previous && current!==previous && !states.transitions[layer]?.[previous]?.includes(current)) return verdict('FAIL','STATE.ILLEGAL_TRANSITION',refs);
    if(layer==='tx' && current==='PT') {
      if(!Number.isFinite(event.physical_power_w)||!Number.isFinite(check.minimum_power_w)) return verdict('INCONCLUSIVE','STATE.PHYSICAL_POWER_MISSING',refs);
      if(event.physical_power_w<check.minimum_power_w) return verdict('FAIL','STATE.LOG_PHYSICAL_MISMATCH',refs);
    }
    if(layer==='battery' && ['CC','CV'].includes(current) && event.battery_observed!==true) return verdict('INCONCLUSIVE','STATE.BATTERY_NOT_OBSERVED',refs);
    if(current==='EPT' && (!event.ept_reason || !check.allowed_ept_reasons?.includes(event.ept_reason))) return verdict('FAIL','STATE.UNEXPECTED_EPT',refs);
    previous=current;time=event.time_s;
  }
  return check.expected_final===previous?verdict('PASS','STATE.SEQUENCE_VALID',refs):verdict('FAIL','STATE.FINAL_MISMATCH',refs);
}
export function communicationRule(check, o) {
  if(!o || !o.evidence_refs?.length || !Array.isArray(o.quality?.issues)||o.quality.issues.length || !o.complete || !o.reference_verified || o.layer!==check.layer || !Number.isInteger(o.expected_count)||o.expected_count<=0) return verdict('INCONCLUSIVE','COMM.REFERENCE_OR_COVERAGE_MISSING');
  if(!Number.isInteger(o.received_count)||o.received_count<0||o.received_count>o.expected_count||!Number.isInteger(o.error_count)||o.error_count<0||o.error_count>o.received_count||!Number.isInteger(o.timeout_count)||o.timeout_count<0) return verdict('INCONCLUSIVE','COMM.INVALID_DENOMINATOR');
  if(!Number.isFinite(check.maximum_error_ratio)||!Number.isFinite(check.maximum_timeouts)) return verdict('INCONCLUSIVE','COMM.LIMIT_MISSING');
  const ratio=(o.expected_count-o.received_count+o.error_count)/o.expected_count;
  return verdict(ratio<=check.maximum_error_ratio&&o.timeout_count<=check.maximum_timeouts&&o.sequence_valid===true?'PASS':'FAIL',ratio>check.maximum_error_ratio?'COMM.FRAME_QUALITY':'COMM.SEQUENCE_OR_TIMEOUT',o.evidence_refs);
}
