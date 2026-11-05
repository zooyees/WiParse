// Additive engineering diagnostics. Never modifies deterministic verdicts.
const tips = [
  [/^RESOURCE\./, '检查工程设置中的角色绑定、序列号和设备连接，重新预检；不要使用临时device_id替代稳定身份。'],
  [/^HOST\./, '检查硬件适配器版本和能力；模拟模式只验证软件流程，不能替代硬件验收。'],
  [/^IO\.|^CLEANUP\.|^RELEASE\./, '确认设备已停止及输出安全；设备状态未知时先人工恢复，禁止直接重试。'],
  [/^LOG\./, '检查串口角色、编码、游标覆盖和缓冲溢出；保留原始字节及采集质量信息。'],
  [/^WAVEFORM\.|^EVIDENCE\./, '检查示波器采集时点、通道、源文件和证据哈希；不要用截图替代缺失的实测数值。'],
  [/^RESET\./, '检查复位线路、电源读回及确认超时；确认安全后重新预检。'],
  [/^CONFIG\.|^PROJECT\.|^RUN\./, '检查当前配置、选中项和冻结来源；修正保存后重新预检，历史证据保留。'],
  [/^ENOSPC$|^EACCES$|^EPERM$|^EIO$|^EROFS$/, '检查磁盘空间、目录权限和存储连接；归档未完成时不能按已完成处理。'],
];
export const recoveryHint = code => tips.find(([pattern]) => pattern.test(code ?? ''))?.[1]
  ?? '根据错误码和原始证据定位原因，确认设备与配置后重新预检；不自动重复输出改变操作。';
export function diagnosis(result) {
  const flatten = rules => (rules ?? []).flatMap(rule => [rule, ...flatten(rule.children)]);
  const issues = flatten(result.results).filter(rule => rule.verdict !== 'PASS');
  return {
    first_error: result.errors?.[0] ?? null,
    first_rule_issue: issues[0] ? {id: issues[0].id, verdict: issues[0].verdict, reason: issues[0].reason} : null,
    rule_issue_count: issues.length,
    recovery_hint: result.errors?.length ? recoveryHint(result.errors[0].code)
      : issues.length ? '检查未满足规则的实际值、限值版本与证据质量；缺测保持不确定，修正后重新测试。' : null,
    automatic_retry: false,
  };
}
export function coveragePlan(frozen) {
  const loops = Number(frozen.options.loop_count ?? 1);
  return frozen.items.map(item => ({item_id: item.config.id, name: item.config.name,
    selected: item.selected, planned: item.selected ? loops : 0, completed: 0,
    pass: 0, fail: 0, inconclusive: 0, error: 0, cancelled: 0}));
}
export function countAttempt(coverage, entry) {
  const item = coverage.find(item => item.item_id === entry.item_id);
  if (!item) throw new Error('Coverage item missing');
  item.completed++;
  if (entry.execution_status === 'completed') item[entry.product_verdict === 'PASS' ? 'pass' : entry.product_verdict === 'FAIL' ? 'fail' : 'inconclusive']++;
  else item[entry.execution_status === 'cancelled' ? 'cancelled' : 'error']++;
}
