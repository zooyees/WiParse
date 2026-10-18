// One source of truth for typed editing, dependency discovery and defensive evaluation.
export const METRICS = Object.freeze({frequency_hz:'Hz',period_s:'s',duty_cycle:'ratio',peak_to_peak_v:'V',minimum_v:'V',maximum_v:'V',mean_v:'V',rise_time_s:'s',fall_time_s:'s'});
export const RULES = Object.freeze({
  log_contains:{label:'日志包含内容',fields:['text']},
  log_regex:{label:'日志正则匹配',fields:['pattern']},
  log_not_regex:{label:'日志禁止匹配',fields:['pattern']},
  log_match_count:{label:'日志匹配次数',fields:['pattern']},
  waveform_metric_range:{label:'波形指标范围',fields:['capture','channel','metric','unit']},
  instrument_measure_range:{label:'仪器测量范围',fields:['unit','resource_role','metric']},
  file_exists:{label:'证据文件存在',fields:['evidence_ref']},
  artifact_hash:{label:'证据哈希一致',fields:['evidence_ref','sha256']},
  all:{label:'全部条件满足',group:true},any:{label:'任一条件满足',group:true},not:{label:'条件取反',group:true}
});
export function flattenRules(rules) { return (rules??[]).flatMap(r=>[r,...flattenRules(r.rules)]); }
export function ruleErrors(r, pointer='/verdict/rules', depth=0) {
  const errors=[], add=(field,message)=>errors.push({path:pointer+(field?'/'+field:''),code:'RULE.INVALID',message});
  if(!r || typeof r!=='object') {add('','规则必须为对象');return errors;}
  if(depth>12) {add('','组合规则最多嵌套12层');return errors;}
  const spec=RULES[r.type];
  if(!spec) {add('type','不支持的规则类型');return errors;}
  if(!/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(r.id??'')) add('id','请输入有效的规则标识');
  if(typeof r.required!=='boolean') add('required','请选择规则是否必须通过');
  for(const field of spec.fields??[]) if(typeof r[field]!=='string'||!r[field].trim()) add(field,'此规则的必填字段不能为空');
  if(spec.group) {
    if(!Array.isArray(r.rules)||!r.rules.length||(r.type==='not'&&r.rules.length!==1)) add('rules',r.type==='not'?'取反规则必须且只能包含一个条件':'请至少添加一个子条件');
    else r.rules.forEach((child,i)=>errors.push(...ruleErrors(child,`${pointer}/rules/${i}`,depth+1)));
  } else if(r.rules!==undefined) add('rules','普通规则不能包含子条件');
  if(r.pattern!==undefined) {
    if(r.pattern.length>512) add('pattern','正则表达式最多512字符');
    try {if(new RegExp(r.pattern).test('')) add('pattern','正则不能匹配空字符串');} catch {add('pattern','正则表达式无效');}
  }
  if(r.minimum_matches!==undefined && (!Number.isSafeInteger(r.minimum_matches)||r.minimum_matches<0)) add('minimum_matches','次数必须为非负整数');
  if(r.maximum_matches!==undefined && (!Number.isSafeInteger(r.maximum_matches)||r.maximum_matches<(r.minimum_matches??0))) add('maximum_matches','最大次数不能小于最小次数');
  if(r.type.endsWith('_range')) {
    if(!r.limit_ref && (!Number.isFinite(r.minimum)||!Number.isFinite(r.maximum)||r.minimum>r.maximum)) add('minimum','请填写有效且顺序正确的上下限');
    if(r.type==='waveform_metric_range' && METRICS[r.metric]!==r.unit) add('unit',`该指标的标准单位为 ${METRICS[r.metric]??'未知'}`);
  }
  return errors;
}
export function ruleSchema(properties) {
  const names=Object.keys(RULES);
  return {oneOf:names.map(type=>{
    const spec=RULES[type];
    const keys=['id','type','required','evidence_refs',...(spec.fields??[]),...(type.endsWith('_range')?['minimum','maximum','limit_ref']:[]),...(type.startsWith('log_')&&type!=='log_not_regex'?['minimum_matches','maximum_matches']:[]),...(spec.group?['rules']:[])];
    const fields=Object.fromEntries(keys.filter(k=>properties[k]).map(k=>[k,structuredClone(properties[k])]));
    fields.type={const:type};
    fields.required.default=true;
    for(const key of spec.fields??[]) if(fields[key]?.type==='string') fields[key].minLength=1;
    if(spec.group) fields.rules={type:'array',minItems:1,...(type==='not'?{maxItems:1}:{}),items:{$ref:'#/$defs/rule'}};
    if(type.endsWith('_range')) {fields.minimum.default=0;fields.maximum.default=1;}
    if(type==='waveform_metric_range') fields.unit.enum=[...new Set(Object.values(METRICS))];
    return {type:'object',title:type,'x-title-zh':spec.label,properties:fields,required:['id','type','required',...(spec.fields??[]),...(spec.group?['rules']:[])],additionalProperties:false,...(type.endsWith('_range')?{allOf:[{if:{not:{required:['limit_ref']}},then:{required:['minimum','maximum']}}]}:{})};
  })};
}
