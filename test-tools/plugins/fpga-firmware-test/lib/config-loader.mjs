import fs from 'node:fs';
import path from 'node:path';
import { digest, hash, jailed, fail } from './common.mjs';
import { inspectFirmware } from './firmware.mjs';
import { ruleErrors } from './rule-contract.mjs';
import {resolveProjectCase} from './project-config.mjs';
const finite = (n, min = 0) => Number.isFinite(n) && n >= min;
export function validateCase(c) {
  const errors = [], add = (p, message) => errors.push({ path: p, code: 'CONFIG.INVALID', message });
  if (!c || typeof c!=='object') {add('/','配置必须为对象');return errors;}
  if (!['wiparse.fpga-test-case/1','wiparse.fpga-test-case/2'].includes(c.schema_version)) add('/schema_version','Unsupported schema');
  if (!/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(c.id ?? '')) add('/id','Invalid ID');
  if (typeof c.name !== 'string' || !c.name.trim() || c.name.trim().length > 128) add('/name','Required, max 128');
  if (!finite(c.timeout_s, 0.001)) add('/timeout_s','Positive timeout required');
  if (!c.instruments || !c.firmware || !c.reset || !c.log_capture || !c.verdict) { add('/','Missing execution configuration'); return errors; }
  if (c.timing?.anchor !== 'after_reset_release') add('/timing/anchor','Reset release required');
  const resetRole = c.reset.method === 'dc_power' ? c.reset.power_role : c.reset.dongle_role;
  if (!['dc_power','wiparse_dongle'].includes(c.reset.method) || !c.instruments[resetRole]) add('/reset','Invalid reset role');
  if(c.reset.method==='dc_power' && c.instruments[resetRole]?.kind!=='dc_source') add('/reset/power_role','电源复位必须选择直流电源角色');
  if (c.reset.method === 'dc_power' && (!finite(c.reset.off_time_ms) || !['voltage_threshold','command_ack'].includes(c.reset.release_confirmation?.type))) add('/reset','Invalid power confirmation');
  if (c.reset.method === 'wiparse_dongle' && c.reset.hardware !== true) add('/reset/hardware','Hardware reset required');
  if(c.reset.method==='wiparse_dongle' && !['debug_probe','wiparse_dongle'].includes(c.instruments[resetRole]?.kind))add('/reset/dongle_role','请选择具有硬件复位能力的调试器或Dongle角色');
  if (c.log_capture.enabled) {
    const l = c.instruments[c.log_capture.resource_role??'log'];
    if (!l || !['serial_port','wiparse_dongle_uart'].includes(l.kind) || !finite(l.selector?.baud,1)) add('/instruments/log','Invalid log role');
    if (l?.kind === 'serial_port' && !l.selector.port) add('/instruments/log/selector/port','Required');
    if (l?.kind === 'wiparse_dongle_uart' && (!l.selector.dongle_serial || !Number.isInteger(l.selector.uart_channel))) add('/instruments/log','Stable Dongle identity required');
    if (!finite(c.log_capture.start_at_s) || !finite(c.log_capture.duration_s,0.001) || !finite(c.log_capture.max_lines,1)) add('/log_capture','Invalid window/capacity');
  }
  const captures = c.waveform_captures ?? [], ids = new Set();
  for (const w of captures) {
    if (!/^[\w.-]+$/.test(w.id) || ids.has(w.id) || !finite(w.at_s) || !finite(w.timeout_s,0.001) || !w.channels?.length) add('/waveform_captures','Invalid or duplicate capture');
    ids.add(w.id);
  }
  const sorted = [...captures].sort((a,b)=>a.at_s-b.at_s);
  for (let i=0;i<sorted.length;i++) for(let j=i+1;j<sorted.length;j++) {
    if (sorted[i].source_waveform?.enabled && sorted[j].at_s < sorted[i].at_s + sorted[i].timeout_s) errors.push({path:'/waveform_captures',code:'WAVEFORM.SCHEDULE_OVERLAP',message:`${sorted[i].id}/${sorted[j].id}`});
  }
  const end = Math.max(0, c.log_capture.enabled ? c.log_capture.start_at_s+c.log_capture.duration_s : 0, ...captures.map(w=>w.at_s+w.timeout_s));
  if (c.timeout_s <= end + (c.reset.off_time_ms??0)/1000 + (c.reset.release_confirmation?.timeout_ms??2000)/1000) add('/timeout_s','Insufficient reset/capture budget');
  const ruleIds = new Set();
  const visit = (r, pointer, depth=0) => {
    if(depth>12)return;
    if(!r||typeof r!=='object') {add(pointer,'无效规则');return;}
    if (!r.id || ruleIds.has(r.id)) add('/verdict/rules','Duplicate/missing rule ID'); ruleIds.add(r.id);
    if (!['log_contains','log_regex','log_not_regex','log_match_count','waveform_metric_range','instrument_measure_range','file_exists','artifact_hash','all','any','not','custom_adapter'].includes(r.type)) add('/verdict/rules','Unknown rule');
    if (r.pattern) { try { new RegExp(r.pattern); } catch { add('/verdict/rules','Invalid regex'); } }
    if (r.type === 'waveform_metric_range' && (!ids.has(r.capture) || !captures.find(w=>w.id===r.capture)?.channels.includes(r.channel))) add('/verdict/rules','Unknown capture/channel');
    if (typeof r.type==='string' && r.type.endsWith('_range') && (!r.unit || (!r.limit_ref && (!finite(r.minimum,-Infinity) || !finite(r.maximum,-Infinity) || r.minimum>r.maximum)))) add(pointer,'Invalid limits/unit');
    if (r.type === 'custom_adapter') add('/verdict/rules','No custom adapters installed');
    if(Array.isArray(r.rules)) r.rules.forEach((child,i)=>visit(child,`${pointer}/rules/${i}`,depth+1));
  };
  if (!c.verdict.rules?.length) add('/verdict/rules','At least one rule required');
  if(Array.isArray(c.verdict.rules)) c.verdict.rules.forEach((r,i)=>{errors.push(...ruleErrors(r,`/verdict/rules/${i}`));visit(r,`/verdict/rules/${i}`);});
  return errors;
}
export function discover(configDir, repositoryRoot = path.dirname(configDir), projectConfig=null) {
  const names = fs.readdirSync(configDir).filter(n=>!/^[_\.]/.test(n)&&n.endsWith('.json')&&!n.endsWith('.disabled.json'));
  if (names.length > 500) throw fail('CONFIG.MAX_ITEMS','500');
  let total=0; const items=[], errors=[], ids=new Set();
  for (const name of names) {
    try {
      const file=jailed(repositoryRoot,path.join(configDir,name)), size=fs.statSync(file).size; total+=size;
      if(size>1048576 || total>33554432) throw fail('CONFIG.SIZE');
      const bytes=fs.readFileSync(file), raw=JSON.parse(bytes.toString('utf8').replace(/^\uFEFF/,''));
      const resolved=projectConfig?resolveProjectCase(raw,projectConfig):null,config=resolved?.value??raw;
      const issues=validateCase(config); if(issues.length) { errors.push(...issues.map(e=>({...e,file,item_id:config.id}))); continue; }
      if(ids.has(config.id)) throw fail('CONFIG.DUPLICATE_ID',config.id); ids.add(config.id);
      const firmware=inspectFirmware(repositoryRoot,file,config.firmware);
      const resolveLimits = rules => rules.map(r=> {
        const copy=structuredClone(r);
        if(r.limit_ref) {
          const [rel,key]=r.limit_ref.split('#'), source=jailed(repositoryRoot,path.resolve(path.dirname(file),rel));
          if(fs.statSync(source).size>1048576) throw fail('LIMIT.SIZE');
          const bytes=fs.readFileSync(source), document=JSON.parse(bytes), limit=document.entries?.[key];
          if(!limit || !Number.isFinite(limit.minimum)||!Number.isFinite(limit.maximum)||limit.minimum>limit.maximum || limit.unit!==r.unit) throw fail('LIMIT.INVALID',r.id);
          Object.assign(copy,{minimum:limit.minimum,maximum:limit.maximum,unit:limit.unit,limit_source:source,limit_sha256:hash(bytes)});
        } else if(r.type.endsWith('_range')) Object.assign(copy,{limit_source:'inline',limit_sha256:digest(r)});
        if(r.rules) copy.rules=resolveLimits(r.rules); return copy;
      });
      const frozen=structuredClone(config); frozen.verdict.rules=resolveLimits(config.verdict.rules);
      items.push({file,config_sha256:hash(bytes),input_sha256:digest(raw),effective_config_sha256:digest(frozen),raw_config:raw,sources:resolved?.sources,config:frozen,firmware});
    } catch(e) { errors.push({file:name,code:e.code??'CONFIG.INVALID_JSON',message:e.message}); }
  }
  items.sort((a,b)=>(a.config.order??0)-(b.config.order??0)||a.config.name.localeCompare(b.config.name,'en',{numeric:true})||a.config.id.localeCompare(b.config.id));
  return {schema_version:'wiparse.fpga-manifest/1',items,errors};
}
export function unchanged(manifest) {
  for(const i of manifest.items) {
    if(hash(fs.readFileSync(i.file))!==i.config_sha256 || hash(fs.readFileSync(i.firmware.absolute))!==i.firmware.sha256) throw fail('CONFIG.CHANGED',i.config.id);
  }
}
