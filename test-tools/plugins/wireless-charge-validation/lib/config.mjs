import fs from 'node:fs';
import path from 'node:path';
import {createHash} from 'node:crypto';
export const VERSION = 'wiparse.wireless-plugin/1';
export const SYS = Array.from({length:10}, (_,i)=>`SYS-${String(i+1).padStart(2,'0')}`);
export const hash = value => createHash('sha256').update(typeof value === 'string' || Buffer.isBuffer(value) ? value : JSON.stringify(value)).digest('hex');
export function fail(code) { throw new Error(code); }
export function inside(file, root) {
  const rel=path.relative(root,file);
  if(rel.startsWith('..') || path.isAbsolute(rel)) fail('CONFIG.PATH_ESCAPE');
  return file;
}
export function readDocument(root, relative) {
  const file=inside(fs.realpathSync(path.resolve(root,relative)),fs.realpathSync(root));
  const bytes=fs.readFileSync(file);
  if(bytes.length>8*1024*1024) fail('CONFIG.SIZE_BUDGET');
  return {value:JSON.parse(bytes),sha256:hash(bytes),path:file};
}
export function loadBundle(root, args={}) {
  const permitted=new Set(['product','suite','mode','fixture','dut_serial','dut_firmware','service_root','file_prefix','project','data_root']);
  for(const key of Object.keys(args)) if(!permitted.has(key)) fail(`CONFIG.OVERLAY_FORBIDDEN:${key}`);
  const id=v=>{if(!/^[a-zA-Z0-9][a-zA-Z0-9_-]*$/.test(v)) fail('CONFIG.INVALID_ID');return v;};
  const product=id(args.product||'reference-tx'), suite=id(args.suite||'smoke');
  const refs={profile:`products/${product}/profile.json`,limits:`products/${product}/limits.json`,states:`products/${product}/states.json`,station:'station.json',suite:`suites/${suite}.json`,adapters:'catalogs/adapters.json',receivers:'catalogs/receivers.json',cables:'catalogs/cables.json',scenarios:'scenarios/cases.json'};
  const docs=Object.fromEntries(Object.entries(refs).map(([k,v])=>[k,readDocument(root,v)]));
  const b=Object.fromEntries(Object.entries(docs).map(([k,d])=>[k,d.value]));
  b.hashes=Object.fromEntries(Object.entries(docs).map(([k,d])=>[k,d.sha256]));
  b.args={...args,product,suite,mode:args.mode||'dry-run',fixture:id(args.fixture||'gold')};
  if(!['dry-run','replay','live'].includes(b.args.mode)) fail('CONFIG.INVALID_MODE');
  for(const doc of Object.values(docs)) if(doc.value.schema_version!==VERSION || !doc.value.version) fail('CONFIG.SCHEMA_VERSION');
  if(!b.limits.items || !b.station.protection || !b.station.bindings || !b.states.layers || !b.states.transitions) fail('CONFIG.REQUIRED_SECTION');
  for(const l of Object.values(b.limits.items)) {
    if(!l.unit || (l.minimum!==null&&!Number.isFinite(l.minimum)) || (l.maximum!==null&&!Number.isFinite(l.maximum)) || (l.minimum===null&&l.maximum===null) || (l.minimum!==null&&l.maximum!==null&&l.minimum>l.maximum)) fail('CONFIG.INVALID_LIMIT');
  }
  if(!Array.isArray(b.scenarios.cases)||!b.scenarios.cases.length||b.scenarios.cases.length>2000) fail('CONFIG.EMPTY_CASES');
  const ids=new Set();
  for(const c of b.scenarios.cases) {
    if(ids.has(c.id)) fail('CONFIG.DUPLICATE_CASE');ids.add(c.id);
    for(const field of ['id','goal','initial_state','risk','capture_plan','cleanup','recovery']) if(!c[field]) fail(`CONFIG.CASE_FIELD:${field}`);
    if(!c.sys?.length || c.sys.some(x=>!SYS.includes(x)) || !c.pl?.length || !c.checks?.length || !c.roles?.length || !c.required_capabilities?.length) fail('CONFIG.CASE_TRACEABILITY');
    if(!Number.isInteger(c.deadline_ms)||c.deadline_ms<1||c.deadline_ms>3600000) fail('CONFIG.DEADLINE');
    const checks=new Set();
    for(const check of c.checks) {
      if(!check.id||checks.has(check.id)) fail('CONFIG.DUPLICATE_CHECK');checks.add(check.id);
      if(!['scalar','state','communication'].includes(check.kind)||!check.required) fail('CONFIG.CHECK_KIND');
      if(check.kind==='scalar'&&!b.limits.items[check.limits_ref]) fail('CONFIG.MISSING_LIMIT');
    }
  }
  if(!b.suite.case_ids?.length || new Set(b.suite.case_ids).size!==b.suite.case_ids.length || b.suite.case_ids.some(x=>!ids.has(x))) fail('CONFIG.SUITE_CASES');
  for(const c of b.scenarios.cases.filter(c=>c.mandatory)) if(!b.suite.case_ids.includes(c.id)) fail('CONFIG.MANDATORY_CASE_OMITTED');
  b.cases=b.suite.case_ids.map(id=>b.scenarios.cases.find(c=>c.id===id));
  return b;
}
export function preflightBundle(b, capabilities={implemented:[],industrial_approved:false}) {
  const blocking=[];
  if(b.args.mode==='live') {
    // SW-01/03 and DG capture are not integrated yet. No legacy bypass or speculative power command.
    blocking.push('INTEGRATION.LIVE_BACKEND_NOT_VALIDATED');
    const required=new Set(b.cases.flatMap(c=>c.required_capabilities));
    for(const c of required) if(!capabilities.implemented?.includes(c)) blocking.push(`CAPABILITY.MISSING:${c}`);
    if(!b.limits.approved || b.limits.simulation_only) blocking.push('LIMITS.NOT_PRODUCT_APPROVED');
    if(!b.station.protection.verified || !b.station.protection.stop_sequence_approved) blocking.push('SAFETY.NOT_VERIFIED');
    for(const role of new Set(b.cases.flatMap(c=>c.roles))) if(!b.station.bindings[role]?.verified) blocking.push(`DEVICE.NOT_VERIFIED:${role}`);
    if(b.suite.industrial && (!capabilities.industrial_approved || !b.station.g4_approved)) blocking.push('ACCEPTANCE.G4_PENDING');
    if(!b.args.dut_serial||!b.args.dut_firmware) blocking.push('DUT.IDENTITY_MISSING');
  }
  return {ok:blocking.length===0,mode:b.args.mode,industrial_approved:false,blocking,hashes:b.hashes,case_count:b.cases.length,product_verdict:'INCONCLUSIVE'};
}
