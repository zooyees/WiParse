import fs from 'node:fs';
import path from 'node:path';
export const escape = value => String(value??'').replaceAll('&','&amp;').replaceAll('<','&lt;').replaceAll('>','&gt;').replaceAll('|','&#124;').replaceAll('\n',' ');
const flatten = rules => rules.flatMap(r=>[r,...flatten(r.children??[])]);
const rows = rules => flatten(rules).map(r=>`|${escape(r.id)}|${escape(r.measured)}|${escape(r.minimum)}|${escape(r.maximum)}|${escape(r.unit)}|${escape(r.verdict)}|${escape(r.source)}|${escape(r.reason)}|${escape((r.evidence_refs??[]).join(', '))}|`).join('\n');
export function beginReport(file, frozen, runId) {
  fs.mkdirSync(path.dirname(file),{recursive:true});
  fs.writeFileSync(file,`# FPGA验证${frozen.options.execution_mode==='simulation'?' · 模拟演示':''}\n\n工程验证报告。执行状态与产品判定分别记录；工业审批状态：未批准。\n\n运行 ID：${runId}\n\n|身份|本轮冻结值|来源|\n|---|---|---|\n`+Object.entries(frozen.identity).filter(([k])=>k!=='field_sources').map(([k,v])=>`|${escape(k)}|${escape(v)}|${escape(frozen.identity.field_sources?.[k])}|`).join('\n')+'\n\n');
  if(frozen.items)fs.appendFileSync(file,`## 测试计划\n\n本轮选择 ${frozen.items.filter(item=>item.selected).length}/${frozen.items.length} 项，计划 ${Number(frozen.options.loop_count??1)} 轮。未选择、未执行与已完成分别记录；本报告不将未测项目当作通过。\n\n`);
  if(frozen.project_config)fs.appendFileSync(file,`## 工程共享来源\n\n工程：${escape(frozen.project_config_snapshot?.value?.name)}；版本：${frozen.project_config.revision}；SHA-256：${frozen.project_config.sha256}\n\nDUT适用约束及仪器绑定来自冻结工程快照；本轮身份来源逐字段标记为用户输入或明确的模拟填充。\n\n`);
}
export function appendReport(file,result,dir,name) {
  const target=path.relative(path.dirname(file),dir).split(path.sep).map(encodeURIComponent).join('/');
  const links=[`[完整结果与证据](${target}/result.json)`];
  const images=[];
  for(const [id,a] of Object.entries(result.evidence.artifacts??{})) {
    const reference=`[${escape(id)}](${target}/${a.path.split('/').map(encodeURIComponent).join('/')})`;
    if(/\.(png|jpe?g|gif|webp|bmp)$/i.test(a.path)) images.push(`!${reference}`);
    else links.push(reference);
  }
  if(fs.existsSync(path.join(dir,'logs','serial.log'))) links.push(`[串口日志](${target}/logs/serial.log)`);
  fs.appendFileSync(file,`## ${escape(name)}\n\n${result.attempt_id} · ${result.execution_status} / ${result.product_verdict}\n\n${links.join(' · ')}\n\n限值 SHA-256：${result.limits_sha256}\n\n|规则|测量|下限|上限|单位|结论|限值来源|原因|证据引用|\n|---|---|---|---|---|---|---|---|---|\n${rows(result.results)}\n\n`);
  if(result.diagnostics?.recovery_hint || result.errors?.length) {
    fs.appendFileSync(file,`### 异常与处置\n\n${(result.errors??[]).map(error=>`- ${escape(error.code)}：${escape(error.message)}`).join('\n')}\n\n${escape(result.diagnostics?.recovery_hint)}\n\n`);
  }
  if(images.length)fs.appendFileSync(file,`### 波形截图\n\n${images.join('\n\n')}\n\n`);
  if(result.project_config)fs.appendFileSync(file,`共享配置 SHA-256：${result.project_config.sha256}；有效配置 SHA-256：${result.effective_config_sha256}\n\n|实际角色|来源|实际绑定|\n|---|---|---|\n`+Object.entries(result.resolved_instruments??{}).map(([role,device])=>`|${escape(role)}|${escape(result.config_sources?.instruments?.[role])}|${escape(JSON.stringify({kind:device.kind,serial:device.serial,port:device.port,dongle_serial:device.dongle_serial,uart_channel:device.uart_channel,selector:device.selector}))}|`).join('\n')+'\n\n');
}

export function appendCoverage(file,suite) {
  const reasons={plan_completed:'计划执行完毕',operator_stop:'操作员停止',execution_error:'执行异常',recovery_required:'设备状态未知，等待安全恢复',stop_on_fail:'失败即停',duration_limit:'时长上限到达，不再启动新项'};
  fs.appendFileSync(file,`
## 执行覆盖

结束原因：${reasons[suite.stop_reason]??escape(suite.stop_reason)}。未执行项不等于通过；未选中项不属于本轮计划。

|测试项|选择|计划次数|已归档次数|通过|失败|不确定|错误|停止|未执行|
|---|---|---|---|---|---|---|---|---|---|
`+
    (suite.coverage??[]).map(item=>`|${escape(item.name)} (${escape(item.item_id)})|${item.selected?'已选择':'未选择'}|${item.planned}|${item.completed}|${item.pass}|${item.fail}|${item.inconclusive}|${item.error}|${item.cancelled}|${Math.max(0,item.planned-item.completed)}|`).join('\n')+'\n');
}
