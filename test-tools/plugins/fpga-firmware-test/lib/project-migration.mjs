import {canonical,checkedProject,resolveProjectCase,validateProjectValue,CASE_SCHEMA,PROJECT_SCHEMA} from './project-config.mjs';
import {hash} from './common.mjs';

// Pure preview: the host owns locking, backups, CAS, transactions and explicit commit.
export function planProjectMigration({cases,projectConfig}){
 const conflicts=[],items=[],role_mapping=[],source_differences=[];
 const issue=(path,message,values=[])=>conflicts.push({path,message,values});
 if(!Array.isArray(cases)||!cases.length||cases.length>500)return {ok:false,conflicts:[{path:'/',message:'需要1至500个测试项'}],project_value:null,items:[]};
 const seen=new Set();for(const c of cases)if(!c||typeof c.id!=='string'||seen.has(c.id))issue('/id','测试项标识缺失或重复');else seen.add(c.id);
 let saved=null;
 if(projectConfig)try{saved=checkedProject(projectConfig);}catch(e){issue('/project_config',e.message);}
 const legacy=cases.filter(c=>c?.schema_version==='wiparse.fpga-test-case/1');
 for(const c of cases)if(!['wiparse.fpga-test-case/1',CASE_SCHEMA].includes(c?.schema_version))issue('/schema_version','不支持的测试项格式',[{id:c?.id,value:c?.schema_version}]);
 if(!legacy.length){
  if(!saved)issue('/project_config','已升级的测试项需要已保存的工程配置');
  else for(const c of cases)try{resolveProjectCase(c,saved);}catch(e){issue(`/items/${c.id}`,e.message);}
  return {ok:!conflicts.length,conflicts,project_value:saved?.value??null,items:[],role_mapping,source_differences};
 }
 const group=(entries,select)=>{
  const map=new Map();for(const entry of entries){const value=select(entry),key=canonical(value??null);if(!map.has(key))map.set(key,{value,ids:[]});map.get(key).ids.push(entry.id);}return [...map.values()];
 };
 const dutGroups=group(legacy,c=>c.dut);
 if(dutGroups.length!==1||!dutGroups[0]?.value)issue('/dut','旧测试项DUT约束不一致，不能自动覆盖',dutGroups);
 const instrumentGroups=group(legacy,c=>c.instruments);if(instrumentGroups.length>1)source_differences.push({path:'/instruments',groups:instrumentGroups});
 const proposedRoles=new Map();
 for(const c of legacy){
  if(!c.instruments||typeof c.instruments!=='object'){issue(`/items/${c.id}/instruments`,'旧仪器绑定缺失');continue;}
  for(const[from,spec]of Object.entries(c.instruments)){
   let to=from;
   if(from==='log'){
    if(spec?.kind==='serial_port')to='log_com';
    else if(spec?.kind==='wiparse_dongle_uart')to='log_dongle';
    else{issue(`/items/${c.id}/instruments/log`,'不支持的旧日志设备类型');continue;}
   }
   if(!proposedRoles.has(to))proposedRoles.set(to,[]);proposedRoles.get(to).push({id:c.id,from,spec});
   role_mapping.push({id:c.id,from,to,usage:from==='log'?'log_capture.resource_role':c.reset?.power_role===from?'reset.power_role':c.reset?.dongle_role===from?'reset.dongle_role':'shared_role'});
  }
 }
 const instruments={};
 for(const[role,entries]of proposedRoles){
  const variants=group(entries,e=>e.spec);
  if(variants.length!==1)issue(`/instruments/${role}`,'同一工程角色有不同绑定，必须显式解决',variants);
  else if(!variants[0].value||typeof variants[0].value!=='object')issue(`/instruments/${role}`,'旧仪器角色无效');
  else instruments[role]=structuredClone(variants[0].value);
 }
 const project_value=saved?structuredClone(saved.value):{schema_version:PROJECT_SCHEMA,name:'FPGA 测试工程',description:'',dut:structuredClone(dutGroups[0]?.value??{}),instruments:{}};
 if(saved&&dutGroups.length===1&&canonical(saved.value.dut)!==canonical(dutGroups[0].value))issue('/dut','已保存工程与旧测试项DUT约束冲突',[{id:'project_config',value:saved.value.dut},...dutGroups]);
 for(const[role,spec]of Object.entries(instruments)){
  const existing=project_value.instruments[role];
  if(existing&&canonical(existing)!==canonical(spec))issue(`/instruments/${role}`,'已保存工程角色与迁移候选不同',[{id:'project_config',value:existing},{id:'legacy',value:spec}]);
  else project_value.instruments[role]=structuredClone(spec);
 }
 for(const e of validateProjectValue(project_value))issue(e.path,e.message);
 for(const c of cases){
  const value=structuredClone(c);
  if(c.schema_version==='wiparse.fpga-test-case/1'){
   value.schema_version=CASE_SCHEMA;delete value.dut;delete value.instruments;
   if(value.log_capture?.enabled){const mapping=role_mapping.find(r=>r.id===c.id&&r.from==='log');if(!mapping)issue(`/items/${c.id}/log_capture`,'日志启用但缺少可迁移的角色');else value.log_capture.resource_role=mapping.to;}
  }
  items.push({id:c.id,value});
 }
 if(!conflicts.length){
  const body={schema_version:'wiparse.project-config/1',project_id:'migration-preview',test_id:'preview',plugin_id:'fpga-firmware-test',revision:1,schema_hash:'0'.repeat(64),value:project_value},record={...body,sha256:hash(canonical(body))};
  for(const item of items)try{resolveProjectCase(item.value,record);}catch(e){issue(`/items/${item.id}`,e.message);}
 }
 return {ok:!conflicts.length,conflicts,project_value:conflicts.length?null:project_value,items:conflicts.length?[]:items,role_mapping,source_differences};
}
