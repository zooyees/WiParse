const finite = Number.isFinite;
const refs = event => Array.isArray(event?.evidence_refs) && event.evidence_refs.length > 0;
const usable = e => refs(e) && Array.isArray(e.quality?.issues) && e.quality.issues.length === 0;

function episodes(events, check) {
  const result = [];
  let previous, start, observedStart=false;
  for (const e of events) {
    if (!usable(e) || typeof e.tx !== 'string' || !e.tx || !finite(e.time_s) || (previous && e.time_s < previous.time_s)) {
      if (start) result.push({start_s:observedStart?start.time_s:null,end_s:null,duration_s:null,classification:'capture_gap',evidence_refs:start.evidence_refs});
      previous = undefined;
      start = undefined;
      continue;
    }
    if (e.tx === 'PT' && previous?.tx !== 'PT') {start=e;observedStart=Boolean(previous?.tx);}
    if (previous?.tx === 'PT' && e.tx && e.tx !== 'PT') {
      const reason=e.tx === 'EPT' ? e.ept_reason ?? null : null;
      const expected=reason && Array.isArray(check.allowed_ept_reasons) ? check.allowed_ept_reasons.includes(reason) : null;
      const knownStart=start && observedStart;
      result.push({start_s:knownStart?start.time_s:null,end_s:e.time_s,
        duration_s:knownStart?e.time_s-start.time_s:null,
        classification:expected===true?'expected_ept':expected===false?'unexpected_ept':'transfer_ended_cause_unknown',
        reported_ept_reason:reason,physical_power_before_w:finite(previous.physical_power_w)?previous.physical_power_w:null,
        physical_power_after_w:finite(e.physical_power_w)?e.physical_power_w:null,
        evidence_refs:[...new Set([...previous.evidence_refs,...e.evidence_refs])]});
      start=undefined;
    }
    previous=e;
  }
  if (start) result.push({start_s:observedStart?start.time_s:null,end_s:null,duration_s:null,classification:'end_not_observed',evidence_refs:start.evidence_refs});
  return result;
}

export function interpret(check, observation, limit) {
  if (!observation) return {missing:['observation'],next_measurement:'按配置采集该检查的原始窗口。'};
  if(check.kind==='state') return {
    layer:check.layer,episodes:check.layer==='tx' ? episodes(Array.isArray(observation.events)?observation.events:[],check) : [],
    alternative:'拔出、协议重协商、接收端 EPT 与保护停机均可引起停充。',
    next_measurement:'对齐输入协商、EPT 原因、桥压与输出功率，核对停充前后时序。'
  };
  if(check.kind==='communication') {
    const {expected_count:n,received_count:r,error_count:e,timeout_count:t}=observation;
    const eligible=usable(observation) && observation.complete===true && observation.reference_verified===true &&
      check.layer===observation.layer && [n,r,e,t].every(Number.isSafeInteger) && n>0 && r>=0 && r<=n && e>=0 && e<=r && t>=0;
    return {layer:check.layer,reference_verified:observation.reference_verified===true,valid_denominator:eligible?n:null,
      expected_count:n??null,received_count:r??null,error_count:e??null,timeout_count:t??null,
      valid_received_fraction:eligible?(r-e)/n:null,
      alternative:'参考端缺帧或采集丢样会改变通信统计的分母。',
      next_measurement:'同步记录发送参考与实际接收序列，区分丢帧、坏帧和重复帧。'};
  }
  const m=observation.measurement;
  const matched=m && m.metric_id===check.metric_id && m.channel===check.channel && observation.state===check.state && m.unit===limit?.unit;
  const comparable=matched && usable(m) && finite(m.value) && finite(m.uncertainty) && m.uncertainty>=0;
  const metric=check.metric_id;
  const gate=metric.startsWith('deadtime_');
  const frequency=metric==='frequency_median';
  const rail=['VIN','VBRG'].includes(check.channel);
  return {metric,channel:check.channel,expected_state:check.state,observed_state:observation.state??null,
    configured_window_s:check.window_s??null,observed_window_s:m?.window??null,
    observed_interval:comparable?[m.value-m.uncertainty,m.value+m.uncertainty]:null,unit:m?.unit??null,
    alternative:gate?'探头时延差和阈值偏差可改变死区测量。':frequency?'PING 与传能阶段混窗会改变频率分布。':
      rail?'适配器合同变化、输入压降和桥级调节都可能改变母线电压。':'只有综合标记时不能确认适配器型号、协商合同或故障部位。',
    next_measurement:gate?'对同一次捕获的互补栅极校正探头时延，分别测双向换向并检查同时导通。':
      frequency?'用状态边界切分 PING 与工作窗口，复核有效周期数、采样间隔和脉冲缺口。':
      rail?'同步采集 VIN、VBRG、输入电流及控制目标，标出协议重协商窗口。':'补录适配器型号、线缆、协商合同、实测电压电流及持续传能区间。'};
}
