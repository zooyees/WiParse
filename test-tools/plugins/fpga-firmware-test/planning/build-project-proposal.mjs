// Stage A artifacts only. Does not alter active schemas, case files or runtime declarations.
import fs from 'node:fs';
import path from 'node:path';
import {fileURLToPath} from 'node:url';
import {createHash} from 'node:crypto';
const here=path.dirname(fileURLToPath(import.meta.url));
const plugin=path.dirname(here),repo=path.resolve(plugin,'../../..');
const read=file=>JSON.parse(fs.readFileSync(file,'utf8'));
const hash=bytes=>createHash('sha256').update(bytes).digest('hex');
const canonical=v=>JSON.stringify(v&&typeof v==='object'?Array.isArray(v)?v.map(x=>JSON.parse(canonical(x))):Object.fromEntries(Object.keys(v).sort().map(k=>[k,JSON.parse(canonical(v[k]))])):v);
const old=read(path.join(plugin,'schemas/fpga-test-case.legacy.schema.json'));
const oldUi=read(path.join(plugin,'schemas/project.ui.json'));
const clone=structuredClone;
const instruments=clone(old.properties.instruments);
const logBranches=instruments.properties.log.oneOf;
instruments.properties.log_com=clone(logBranches.find(v=>v.properties.kind.const==='serial_port'));
instruments.properties.log_dongle=clone(logBranches.find(v=>v.properties.kind.const==='wiparse_dongle_uart'));
for(const[name,zh]of [['log_com','COM 日志角色'],['log_dongle','Dongle UART 日志角色']]){instruments.properties[name].title=name;instruments.properties[name]['x-title-zh']=zh;}
const schema={$schema:old.$schema,title:'FPGA project configuration','x-title-zh':'FPGA 工程共享配置',type:'object',properties:{schema_version:{const:'wiparse.fpga-project/1'},name:{type:'string',minLength:1,maxLength:128,title:'Project name','x-title-zh':'工程名称'},description:{type:'string',maxLength:2048,title:'Description','x-title-zh':'工程说明'},dut:clone(old.properties.dut),instruments},required:['schema_version','name','dut','instruments'],additionalProperties:false};
const fields=Object.fromEntries(Object.entries(oldUi.fields).filter(([p])=>p==='dut'||p.startsWith('dut.')||p==='instruments'||p.startsWith('instruments.')));
for(const role of ['log_com','log_dongle'])for(const[p,value]of Object.entries(oldUi.fields))if(p.startsWith('instruments.log.'))fields[p.replace('instruments.log.',`instruments.${role}.`)]=clone(value);
Object.assign(fields,{schema_version:{hidden:true},name:{label:'Project name',label_zh:'工程名称',width:'long',order:0},description:{label:'Description',label_zh:'工程说明',width:'long',order:10},dut:{...fields.dut,description_zh:'当前工程的适用约束。实际序列号、操作员等在本轮参数中填写；不以工程默认值覆盖实际输入。'}});
const ui={schema_version:'wiparse.config-ui/1',source_view:'read_only',groups:[{label:'Project',label_zh:'工程基本信息',fields:['name','description']},{label:'DUT requirements',label_zh:'DUT 身份约束',fields:['dut']},{label:'Resource bindings',label_zh:'仪器绑定',fields:['instruments']}],fields,widgets:Object.fromEntries(Object.entries(oldUi.widgets).filter(([p])=>p.startsWith('instruments.'))),conditional_fields:oldUi.conditional_fields.filter(v=>v.path.startsWith('instruments.')),help_zh:'阶段A声明草案，尚未启用。共享数据按project/test/plugin隔离，保存后才可预检；单项不覆盖共享绑定。'};
for(const role of ['log_com','log_dongle'])ui.widgets[`instruments.${role}.selector.${role==='log_com'?'port':'dongle_serial'}`]=role==='log_com'?{type:'serial_port_selector'}:{type:'instrument_selector',kind:'wiparse_dongle_uart',value_field:'dongle_serial'};
const casesDir=path.join(repo,'recipes/fpga-firmware-test/cases');
const cases=fs.readdirSync(casesDir).filter(f=>f.endsWith('.json')).sort().map(file=>{const bytes=fs.readFileSync(path.join(casesDir,file));return {file:`recipes/fpga-firmware-test/cases/${file}`,sha256:hash(bytes),value:JSON.parse(bytes)};});
const groups=select=>{
 const result=new Map();for(const c of cases){const value=select(c.value),key=canonical(value);if(!result.has(key))result.set(key,{value,ids:[]});result.get(key).ids.push(c.value.id);}
 return [...result.values()].map(g=>({...g,count:g.ids.length}));
};
const flatten=(o,p='',out={})=>{for(const[k,v]of Object.entries(o)){const q=p?`${p}.${k}`:k;if(v&&typeof v==='object'&&!Array.isArray(v))flatten(v,q,out);else out[q]=v;}return out;};
const leaves=cases.map(c=>flatten({dut:c.value.dut,instruments:c.value.instruments}));
const paths=[...new Set(leaves.flatMap(v=>Object.keys(v)))].sort();
const conflicts=paths.map(p=>({path:p,variants:groups(c=>{const f=flatten({dut:c.dut,instruments:c.instruments});return Object.hasOwn(f,p)?{present:true,value:f[p]}:{present:false};})})).filter(g=>g.variants.length>1);
const audit={stage:'A-read-only',total:cases.length,distinct_ids:new Set(cases.map(c=>c.value.id)).size,distinct_names:new Set(cases.map(c=>c.value.name)).size,sources:cases.map(({file,sha256})=>({file,sha256})),dut_groups:groups(c=>c.dut),instrument_groups:groups(c=>c.instruments),reset_groups:groups(c=>c.reset.method),timeout_groups:groups(c=>c.timeout_s),conflicts,automatic_migration_allowed:conflicts.length===0,shared_candidate:conflicts.length?null:{dut:cases[0]?.value.dut,instruments:cases[0]?.value.instruments},role_reference_extension:'proposal_pending_architecture_confirmation',sample_scope:'DEMO fixtures only; no normalization of user configurations'};
for(const[name,value]of [['project.schema.proposal.json',schema],['project.ui.proposal.json',ui],['legacy-config-differences.json',audit]])fs.writeFileSync(path.join(here,name),JSON.stringify(value,null,2)+'\n');
console.log(JSON.stringify({stage:'A',cases:cases.length,dut_groups:audit.dut_groups.length,instrument_groups:audit.instrument_groups.length,conflicting_leaf_paths:conflicts.length,automatic_migration_allowed:audit.automatic_migration_allowed}));
