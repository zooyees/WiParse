import { fail } from './common.mjs';
export function resolveResources(roles, devices) {
  const result = {};
  for (const [role, spec] of Object.entries(roles)) {
    if ('device_id' in spec || 'device_id' in spec.selector) throw fail('RESOURCE.UNSTABLE_ID', role);
    const matches = devices.filter(d => d.kind === spec.kind && Object.entries(spec.selector).every(([k,v]) => ['baud','data_bits','stop_bits','parity','flow_control'].includes(k) || d[k] === v));
    if (matches.length !== 1 && spec.required !== false) throw fail(matches.length ? 'RESOURCE.AMBIGUOUS' : 'RESOURCE.MISSING', role);
    if (matches.length === 1) result[role] = {...structuredClone(matches[0]),selector:structuredClone(spec.selector)};
  }
  const dongles = Object.values(result).map(d=>d.dongle_serial).filter(Boolean);
  if (new Set(dongles).size > 1) throw fail('RESOURCE.DONGLE_MISMATCH');
  return result;
}
