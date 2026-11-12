import fs from 'node:fs';
import path from 'node:path';
import {hash,SYS} from './config.mjs';
import {interpret} from './domain-evidence.mjs';
import {aggregate} from './rules.mjs';
export function coverage(bundle, cases) {
  return SYS.map(id=>{
    const required=bundle.scenarios.cases.filter(c=>c.sys.includes(id));
    const actual=cases.filter(c=>c.sys.includes(id));
    return {sys_id:id,required_case_ids:required.map(c=>c.id),selected_case_ids:actual.map(c=>c.case_id),missing_case_ids:required.filter(c=>!actual.some(x=>x.case_id===c.id)).map(c=>c.id),verdict:actual.length===required.length?aggregate(actual):'INCONCLUSIVE'};
  });
}
export function writeReport(dir, bundle, run) {
  fs.mkdirSync(dir,{recursive:true});
  const documents={'checks.json':run,'coverage.json':coverage(bundle,run.cases),'configuration.json':{schema_version:bundle.profile.schema_version,hashes:bundle.hashes,profile:bundle.profile,limits:bundle.limits,station:bundle.station,suite:bundle.suite,states:bundle.states,catalogs:{adapters:bundle.adapters,receivers:bundle.receivers,cables:bundle.cables},scenarios:bundle.scenarios}};
  for(const [name,value] of Object.entries(documents)) fs.writeFileSync(path.join(dir,name),JSON.stringify(value,null,2)+'\n');
  fs.writeFileSync(path.join(dir,'errors.jsonl'),run.errors.map(e=>JSON.stringify(e)).join('\n')+(run.errors.length?'\n':''));
  const cell=value=>String(value??'').replaceAll('\\','\\\\').replaceAll('|','\\|').replace(/[\r\n]+/g,' ');
  const caseRows=run.cases.map((c,i)=>{
    const name=`case-${String(i+1).padStart(3,'0')}.json`;
    fs.writeFileSync(path.join(dir,name),JSON.stringify({run_id:run.run_id,attempt_id:run.attempt_id,...c},null,2)+'\n');
    return `| ${cell(c.case_id)} | ${cell(c.execution_status)} | ${cell(c.product_verdict)} | ${cell(c.reason_code)} | [检查数据](${name}) |`;
  });
  const replayPath = path.join(dir, 'replay-input.json');
  const replay = run.fixture_hash && fs.existsSync(replayPath) ? JSON.parse(fs.readFileSync(replayPath, 'utf8')) : null;
  const pointer = value => String(value).replaceAll('~', '~0').replaceAll('/', '~1');
  const investigations = [];
  const domain=[];
  const adapters=[];
  const definitions=new Map(bundle.scenarios.cases.flatMap(c=>c.checks.map(check=>[JSON.stringify([c.id,check.id]),check])));
  for (const c of run.cases) {
    for (const check of c.checks) {
      const observed = replay?.cases?.[c.case_id]?.[check.check_id];
      const definition=definitions.get(JSON.stringify([c.case_id,check.check_id]));
      const details=interpret(definition,observed,bundle.limits.items[definition.limits_ref]);
      domain.push({case_id:c.case_id,check_id:check.check_id,scope:check.scope,details});
      if(c.sys.includes('SYS-01')){
        const adapter=observed?.adapter, contract=observed?.contract;
        const evidence=Array.isArray(check.evidence_refs)?check.evidence_refs:[];
        adapters.push({case_id:c.case_id,check_id:check.check_id,scope:check.scope,
          adapter_id:adapter?.id??null,adapter_serial:adapter?.serial??null,cable_id:observed?.cable_id??null,
          protocol:contract?.protocol??null,negotiated_voltage_v:Number.isFinite(contract?.voltage_v)?contract.voltage_v:null,
          negotiated_current_a:Number.isFinite(contract?.current_a)?contract.current_a:null,
          transfer_duration_s:Number.isFinite(observed?.transfer_duration_s)&&observed.transfer_duration_s>=0?observed.transfer_duration_s:null,
          evidence_refs:evidence,check_verdict:check.product_verdict,
          identity_and_contract_observed:Boolean(adapter?.serial && contract?.protocol && Number.isFinite(contract?.voltage_v) && evidence.length),
          industrial_approved:run.industrial_approved});
      }
      if(check.product_verdict==='PASS') continue;
      investigations.push({
        case_id: c.case_id, check_id: check.check_id, verdict: check.product_verdict,
        reason: check.reason_code, scope: check.scope, state: check.state_window,
        measurement: check.measurement ? {value:check.measurement.value, unit:check.measurement.unit,
          channel:check.measurement.channel,window:check.measurement.window,uncertainty:check.measurement.uncertainty} : null,
        evidence_refs: check.evidence_refs ?? [], limits_hash: check.limits_hash,
        observation_pointer: observed == null ? null : '/cases/' + pointer(c.case_id) + '/' + pointer(check.check_id),
        confirmed_root_cause: null,
        next_measurement:details.next_measurement,alternative:details.alternative??null
      });
    }
  }
  fs.writeFileSync(path.join(dir,'adapter-matrix.json'),JSON.stringify({
    run_id:run.run_id,mode:run.mode,configured_protocols:bundle.adapters.objects.map(adapter=>adapter.protocol).filter(Boolean),
    rows:adapters,industrial_approved:run.industrial_approved},null,2)+'\n');
  fs.writeFileSync(path.join(dir,'investigation.json'), JSON.stringify({
    run_id: run.run_id, attempt_id: run.attempt_id, mode: run.mode,
    replay: replay ? { file: 'replay-input.json', sha256: hash(fs.readFileSync(replayPath)) } : null,
    synchronized_capture:run.synchronized_capture??null,first_error_id: run.first_error_id, findings: investigations,domain_analysis:domain
  }, null, 2) + '\n');
  let recordingLink='';
  if(run.mode==='recorded' && replay) {
    // Keep station text out of the Markdown parser. JSON preserves original quotes,
    // newlines, offsets and units even when captured text looks like markup.
    const observed=[];
    const references = new Set();
    for(const f of investigations) {
      const refs=f.evidence_refs;
      for(const ref of refs) references.add(ref);
      observed.push({case_id:f.case_id,check_id:f.check_id,reason:f.reason,measurement:f.measurement,evidence_refs:refs});
    }
    fs.writeFileSync(path.join(dir,'recorded-evidence.json'),JSON.stringify({
      station:run.capture,dut:run.dut,run_id:run.run_id,attempt_id:run.attempt_id,observed,
      evidence:Object.fromEntries([...references].map(ref=>[ref,replay.evidence[ref]]))
    },null,2)+'\n');
    recordingLink='；[检查项对应的原文、位置与测量窗口](recorded-evidence.json)';
  }
  const replayLink=fs.existsSync(path.join(dir,'replay-input.json'))?'；[原始回放数据](replay-input.json)':'';
  const text=`# 无线充电自动检验\n\n模式：${cell(run.mode)}；执行：${cell(run.execution_status)}；产品：${cell(run.product_verdict)}\n\n运行 ID：${cell(run.run_id)}；尝试 ID：${cell(run.attempt_id)}\n\n检查结果：${cell(run.validation_verdict)}。工位批准：${cell(run.industrial_approved)}。限值见本次运行的配置记录。\n\n[完整检查记录](checks.json)；[覆盖率](coverage.json)；[适配器与输入合同](adapter-matrix.json)；[配置与限值](configuration.json)；[异常与处置](errors.jsonl)；[逐项诊断与原始证据定位](investigation.json)；[文件校验清单](manifest.json)${replayLink}${recordingLink}\n\n| 用例 | 执行状态 | 判定 | 原因 | 数据 |\n|---|---|---|---|---|\n`+caseRows.join('\n')+'\n\n首次异常：'+cell(run.first_error_id||'无')+'。候选原因不表示已确认根因，处置建议见异常记录。\n';
  const unresolved = investigations.filter(f=>f.reason!=='CAPTURE.OBSERVED_REVIEW_REQUIRED');
  let findings = '';
  if (unresolved.length) {
    findings = '\n## 待调查项\n\n| 用例 / 检查 | 观测值 | 现有证据 | 下一项检查 |\n|---|---|---|---|\n';
    for (const f of unresolved.slice(0,12)) {
      const measured = Number.isFinite(f.measurement?.value) ? `${f.measurement.value} ${f.measurement.unit ?? ''}` : '缺少标量';
      findings += `| ${cell(f.case_id)} / ${cell(f.check_id)} | ${cell(measured)} | ${cell(f.reason)} | ${cell(f.next_measurement)} |\n`;
    }
    if (unresolved.length>12) findings += `\n共 ${unresolved.length} 项，全部记录见上方逐项诊断索引。\n`;
  }
  fs.writeFileSync(path.join(dir,'summary.md'),text+findings);
  const files=fs.readdirSync(dir).filter(n=>n!=='manifest.json').map(name=>({path:name,sha256:hash(fs.readFileSync(path.join(dir,name)))}));
  fs.writeFileSync(path.join(dir,'manifest.json'),JSON.stringify({schema_version:bundle.profile.schema_version,run_id:run.run_id,attempt_id:run.attempt_id,industrial_approved:run.industrial_approved,files},null,2)+'\n');
  return {summary_md:path.join(dir,'summary.md'),artifacts:{items:fs.readdirSync(dir).map(n=>({output:'reports',view:n.endsWith('.md')?'report':'external',path:path.join(dir,n)}))}};
}
