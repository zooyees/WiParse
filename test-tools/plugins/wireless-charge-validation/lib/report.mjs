import fs from 'node:fs';
import path from 'node:path';
import {hash,SYS} from './config.mjs';
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
  const replayLink=fs.existsSync(path.join(dir,'replay-input.json'))?'；[原始回放数据](replay-input.json)':'';
  const text=`# 无线充电自动检验\n\n模式：${cell(run.mode)}；执行：${cell(run.execution_status)}；产品：${cell(run.product_verdict)}\n\n运行 ID：${cell(run.run_id)}；尝试 ID：${cell(run.attempt_id)}\n\n离线验证结论：${cell(run.validation_verdict)}。工业批准：false。示例限值仅用于模拟器验证。\n\n[完整检查记录](checks.json)；[覆盖率](coverage.json)；[配置与限值](configuration.json)；[异常与处置](errors.jsonl)；[文件校验清单](manifest.json)${replayLink}\n\n| 用例 | 执行状态 | 判定 | 原因 | 数据 |\n|---|---|---|---|---|\n`+caseRows.join('\n')+'\n\n首次异常：'+cell(run.first_error_id||'无')+'。候选原因不表示已确认根因，处置建议见异常记录。\n';
  fs.writeFileSync(path.join(dir,'summary.md'),text);
  const files=fs.readdirSync(dir).filter(n=>n!=='manifest.json').map(name=>({path:name,sha256:hash(fs.readFileSync(path.join(dir,name)))}));
  fs.writeFileSync(path.join(dir,'manifest.json'),JSON.stringify({schema_version:bundle.profile.schema_version,run_id:run.run_id,attempt_id:run.attempt_id,industrial_approved:false,files},null,2)+'\n');
  return {summary_md:path.join(dir,'summary.md'),artifacts:{items:fs.readdirSync(dir).map(n=>({output:'reports',view:n.endsWith('.md')?'report':'external',path:path.join(dir,n)}))}};
}
