import {digest,hash,fail} from './common.mjs';
import {flattenRules} from './rule-contract.mjs';

export const PROJECT_PROTOCOL='wiparse.project-config/1';
export const PROJECT_SCHEMA='wiparse.fpga-project/1';
export const CASE_SCHEMA='wiparse.fpga-test-case/2';
export const canonical=value=>JSON.stringify(value,(_key,v)=>v&&typeof v==='object'&&!Array.isArray(v)?Object.fromEntries(Object.keys(v).sort().map(k=>[k,v[k]])):v);
export const projectMetadata=record=>({revision:record.revision,sha256:record.sha256,schema_hash:record.schema_hash});
export function validateProjectValue(value){
 const errors=[],add=(path,message)=>errors.push({path,code:'PROJECT.INVALID',message});
 if(!value||typeof value!=='object'){add('/','请先保存当前工程共享配置');return errors;}
 if(value.schema_version!==PROJECT_SCHEMA)add('/schema_version','不支持的工程共享格式');
 if(typeof value.name!=='string'||!value.name.trim()||value.name.length>128)add('/name','工程名称不能为空，最多128字符');
 if(!value.dut||typeof value.dut!=='object'||Array.isArray(value.dut))add('/dut','请配置工程DUT约束');
 if(!value.instruments||typeof value.instruments!=='object'||Array.isArray(value.instruments)){add('/instruments','请配置工程仪器角色');return errors;}
 for(const[id,spec]of Object.entries(value.instruments)){
  const p=`/instruments/${id}`;
  if(!/^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$/.test(id))add(p,'角色标识无效');
  if(!spec||typeof spec!=='object'||!spec.selector||typeof spec.selector!=='object'||Array.isArray(spec.selector)){add(p,'请填写设备类型及稳定选择条件');continue;}
  if(!['debug_probe','oscilloscope','dc_source','serial_port','wiparse_dongle_uart','wiparse_dongle','dmm','load','bridge'].includes(spec.kind))add(`${p}/kind`,'不支持的设备类型');
  if('device_id'in spec||'device_id'in spec.selector)add(`${p}/selector`,'不可使用运行期device_id作为稳定身份');
  if(spec.kind==='serial_port'&&(typeof spec.selector.port!=='string'||!spec.selector.port.trim()||!Number.isFinite(spec.selector.baud)||spec.selector.baud<=0))add(`${p}/selector`,'COM角色需要串口及有效波特率');
  if(spec.kind==='wiparse_dongle_uart'&&(typeof spec.selector.dongle_serial!=='string'||!spec.selector.dongle_serial.trim()||!Number.isInteger(spec.selector.uart_channel)||spec.selector.uart_channel<0||!Number.isFinite(spec.selector.baud)||spec.selector.baud<=0))add(`${p}/selector`,'Dongle UART需要物理序列号、通道及波特率');
 }
 return errors;
}
export function checkedProject(record,{projectId,testId,pluginId='fpga-firmware-test'}={}){
 if(!record||record.saved===false||!Number.isSafeInteger(record.revision)||record.revision<1)throw fail('PROJECT.REQUIRED','请先保存当前测试工程设置');
 const {sha256,saved,errors,...body}=record;
 if(record.schema_version!==PROJECT_PROTOCOL||record.plugin_id!==pluginId||(projectId&&record.project_id!==projectId)||(testId&&record.test_id!==testId)||typeof record.schema_hash!=='string'||!/^[a-f0-9]{64}$/.test(record.schema_hash)||hash(canonical(body))!==sha256)throw fail('PROJECT.CONTENT_MISMATCH','工程共享配置来源或哈希不匹配');
 const issues=validateProjectValue(record.value);if(issues.length)throw Object.assign(fail('PROJECT.INVALID','工程共享配置无效'),{errors:issues});
 return structuredClone({...body,sha256});
}
export function resolveProjectCase(raw,record){
 const project=checkedProject(record);
 if(raw?.schema_version!==CASE_SCHEMA)throw fail('PROJECT.MIGRATION_REQUIRED','此测试项尚未迁移到工程共享配置');
 if(Object.hasOwn(raw,'dut')||Object.hasOwn(raw,'instruments'))throw fail('PROJECT.ITEM_OVERRIDE','测试项不能覆盖工程DUT或仪器绑定');
 const value=structuredClone(raw),roles=new Set();
 if(value.reset?.method==='dc_power')roles.add(value.reset.power_role);
 if(value.reset?.method==='wiparse_dongle')roles.add(value.reset.dongle_role);
 if(value.waveform_captures?.length)roles.add('scope');
 if(value.log_capture?.enabled){if(!value.log_capture.resource_role)throw fail('PROJECT.ROLE_REQUIRED','请为日志采集选择工程仪器角色');roles.add(value.log_capture.resource_role);}
 for(const rule of flattenRules(Array.isArray(value.verdict?.rules)?value.verdict.rules:[]))if(rule.type==='instrument_measure_range')roles.add(rule.resource_role);
 const instruments={};
 for(const role of roles){if(typeof role!=='string'||!project.value.instruments[role])throw fail('PROJECT.ROLE_MISSING',String(role));instruments[role]=structuredClone(project.value.instruments[role]);}
 if(value.log_capture?.enabled&&!['serial_port','wiparse_dongle_uart'].includes(instruments[value.log_capture.resource_role].kind))throw fail('PROJECT.ROLE_KIND','日志采集必须引用COM或Dongle UART角色');
 if(value.waveform_captures?.length&&instruments.scope.kind!=='oscilloscope')throw fail('PROJECT.ROLE_KIND','波形任务必须引用示波器角色scope');
 if(value.reset?.method==='dc_power'&&instruments[value.reset.power_role]?.kind!=='dc_source')throw fail('PROJECT.ROLE_KIND','电源复位必须引用直流电源角色');
 if(value.reset?.method==='wiparse_dongle'&&!['debug_probe','wiparse_dongle'].includes(instruments[value.reset.dongle_role]?.kind))throw fail('PROJECT.ROLE_KIND','硬件复位必须引用探针或Dongle角色');
 value.dut=structuredClone(project.value.dut);value.instruments=instruments;
 return {value,project_config:projectMetadata(project),sources:{dut:'project_config',instruments:Object.fromEntries([...roles].map(role=>[role,`project_config.instruments.${role}`])),item:'case',identity:'run_parameters'},effective_sha256:digest(value)};
}
