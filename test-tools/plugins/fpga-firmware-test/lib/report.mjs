import fs from 'node:fs';
import path from 'node:path';
export const escape = value => String(value??'').replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('|','&#124;').replaceAll('\n',' ');
const flatten = rules => rules.flatMap(r=>[r,...flatten(r.children??[])]);
const rows = rules => flatten(rules).map(r=>`|${escape(r.id)}|${escape(r.measured)}|${escape(r.minimum)}|${escape(r.maximum)}|${escape(r.unit)}|${escape(r.verdict)}|${escape(r.source)}|`).join('\n');
export function beginReport(file, frozen, runId) {
  fs.mkdirSync(path.dirname(file),{recursive:true});
  fs.writeFileSync(file,`# FPGA固件测试${frozen.options.execution_mode==='simulation'?' · 模拟演示（非硬件验收）':''}\n\n工程可编辑限值 / 未执行正式审批 / industrial_approved=false\n\n固件身份未验证。运行 ID：${runId}\n\n|身份|本轮冻结值|来源|\n|---|---|---|\n`+Object.entries(frozen.identity).filter(([k])=>k!=='field_sources').map(([k,v])=>`|${escape(k)}|${escape(v)}|${escape(frozen.identity.field_sources?.[k])}|`).join('\n')+'\n\n');
  if(frozen.project_config)fs.appendFileSync(file,`## 工程共享来源\n\n工程：${escape(frozen.project_config_snapshot?.value?.name)}；版本：${frozen.project_config.revision}；SHA-256：${frozen.project_config.sha256}\n\nDUT适用约束及仪器绑定来自冻结工程快照；本轮身份来源逐字段标记为用户输入或明确的模拟填充。\n\n`);
}
export function appendReport(file,result,dir,name) {
  const target=path.relative(path.dirname(file),dir).split(path.sep).map(encodeURIComponent).join('/');
  const links=[`[完整结果与证据](${target}/result.json)`];
  for(const [id,a] of Object.entries(result.evidence.artifacts??{})) links.push(`[${escape(id)}](${target}/${a.path.split('/').map(encodeURIComponent).join('/')})`);
  if(fs.existsSync(path.join(dir,'logs','serial.log'))) links.push(`[串口日志](${target}/logs/serial.log)`);
  fs.appendFileSync(file,`## ${escape(name)}\n\n${result.attempt_id} · ${result.execution_status} / ${result.product_verdict}\n\n${links.join(' · ')}\n\n限值 SHA-256：${result.limits_sha256}\n\n|规则|测量|下限|上限|单位|结论|来源|\n|---|---|---|---|---|---|---|\n${rows(result.results)}\n\n`);
  if(result.project_config)fs.appendFileSync(file,`共享配置 SHA-256：${result.project_config.sha256}；有效配置 SHA-256：${result.effective_config_sha256}\n\n|实际角色|来源|实际绑定|\n|---|---|---|\n`+Object.entries(result.resolved_instruments??{}).map(([role,device])=>`|${escape(role)}|${escape(result.config_sources?.instruments?.[role])}|${escape(JSON.stringify({kind:device.kind,serial:device.serial,port:device.port,dongle_serial:device.dongle_serial,uart_channel:device.uart_channel,selector:device.selector}))}|`).join('\n')+'\n\n');
}
