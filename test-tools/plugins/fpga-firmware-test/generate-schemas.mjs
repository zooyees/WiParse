import fs from 'node:fs';
import {ruleSchema} from './lib/rule-contract.mjs';
import {projectSchemas,projectUis} from './lib/project-declarations.mjs';
const dir='test-tools/plugins/fpga-firmware-test/';
const str={type:'string'}, num={type:'number',minimum:0}, bool={type:'boolean'};
const object=(properties,required=Object.keys(properties))=>({type:'object',properties,required,additionalProperties:false});
const selector=object({serial:str,resource:str,manufacturer:str,model:str,dongle_serial:str,uart_channel:{type:'integer',minimum:0},port:str,baud:{type:'integer',minimum:1},data_bits:{type:'integer'},stop_bits:num,parity:{enum:['none','even','odd']},flow_control:str},[]);
const role=object({kind:{enum:['debug_probe','oscilloscope','dc_source','serial_port','wiparse_dongle_uart','wiparse_dongle','dmm','load','bridge']},selector,required:bool},['kind','selector']);
const confirmation={oneOf:[object({type:{const:'voltage_threshold'},minimum_v:num,stable_ms:num,timeout_ms:num}),object({type:{enum:['command_ack','command_complete']},timeout_ms:num})]};
const rule=object({id:str,type:{enum:['log_contains','log_regex','log_not_regex','log_match_count','waveform_metric_range','instrument_measure_range','file_exists','artifact_hash','all','any','not','custom_adapter']},required:bool,text:str,pattern:str,minimum_matches:num,maximum_matches:num,capture:str,channel:str,metric:{enum:['frequency_hz','period_s','duty_cycle','peak_to_peak_v','minimum_v','maximum_v','mean_v','rise_time_s','fall_time_s']},minimum:{type:'number'},maximum:{type:'number'},unit:str,limit_ref:str,evidence_ref:str,evidence_refs:{type:'array',items:str},sha256:{type:'string',pattern:'^[a-fA-F0-9]{64}$'},rules:{type:'array',items:{$ref:'#/$defs/rule'}}},['id','type','required']);
const schema=JSON.parse(JSON.stringify({$schema:'https://json-schema.org/draft/2020-12/schema',...object({schema_version:{const:'wiparse.fpga-test-case/1'},editor_revision:{type:'integer',minimum:0},updated_at:{type:'string',format:'date-time'},id:{type:'string',pattern:'^[A-Za-z0-9][A-Za-z0-9._-]{0,63}$'},name:{type:'string',minLength:1,maxLength:128},enabled:bool,order:{type:'integer',minimum:0},tags:{type:'array',maxItems:32,items:{type:'string',maxLength:64},uniqueItems:true},timeout_s:{type:'number',exclusiveMinimum:0},dut:object({model:str,serial_required:bool,board_revision:str,fpga_model:str,fixture_id:str,station_id:str},[]),instruments:{type:'object',properties:Object.fromEntries(['programmer','scope','power','log','dmm','load','bridge'].map(k=>[k,role])),additionalProperties:false},firmware:object({download:{type:'boolean',description:'接口预留；true 阻断预检'},implementation:{const:'reserved'},path:str,sha256:{type:'string',pattern:'^[a-fA-F0-9]{64}$'},target:str,format:{enum:['hex','bin']},base_address:{type:['integer','null'],minimum:0},verify:bool},['download','path','sha256','target','format']),reset:{oneOf:[object({method:{const:'dc_power'},power_role:str,channel:{type:'integer',minimum:1},off_time_ms:num,release_confirmation:confirmation}),object({method:{const:'wiparse_dongle'},dongle_role:str,hardware:{const:true},release_confirmation:confirmation})]},timing:object({anchor:{const:'after_reset_release'}}),log_capture:object({enabled:bool,start_at_s:num,duration_s:{type:'number',exclusiveMinimum:0},max_lines:{type:'integer',minimum:1},encoding:{const:'utf-8'}}),waveform_captures:{type:'array',items:object({id:{type:'string',pattern:'^[A-Za-z0-9_.-]+$'},at_s:num,kind:{const:'screenshot'},channels:{type:'array',minItems:1,items:str},source_waveform:object({enabled:bool,format:{enum:['instrument_native','csv']}},['enabled']),timeout_s:{type:'number',exclusiveMinimum:0}})},verdict:object({mode:{const:'all'},rules:{type:'array',minItems:1,items:{$ref:'#/$defs/rule'}},ai_assist:object({enabled:bool,policy:{const:'advisory_only'},provider_profile:str,inputs:{type:'array',items:{enum:['log','log_excerpt','rule_results','waveform_metrics']}},required:bool,timeout_s:{type:'number',exclusiveMinimum:0},redaction_profile:str,prompt_template_id:str},['enabled','policy'])},['mode','rules'])},['schema_version','id','name','enabled','timeout_s','dut','instruments','firmware','reset','timing','log_capture','waveform_captures','verdict']),$defs:{rule}}));
schema.properties.reset.oneOf[0].title='DC power reset';schema.properties.reset.oneOf[0]['x-title-zh']='直流电源断电复位';
schema.properties.reset.oneOf[1].title='Dongle hardware reset';schema.properties.reset.oneOf[1]['x-title-zh']='Dongle 硬件复位';
for(const reset of schema.properties.reset.oneOf){
 const branches=reset.properties.release_confirmation.oneOf;
 branches[0].title='Voltage threshold confirmation';branches[0]['x-title-zh']='电压阈值确认';
 branches[1].title='Command acknowledgement confirmation';branches[1]['x-title-zh']='指令应答确认';
 for(const branch of branches){branch.properties.timeout_ms.title='Confirmation timeout';branch.properties.timeout_ms['x-title-zh']='确认超时';}
}
for(const[kind,cn,en,required]of[['serial_port','普通 COM 串口','COM serial port',['port','baud']],['wiparse_dongle_uart','WiParse Dongle UART','WiParse Dongle UART',['dongle_serial','uart_channel','baud']]]){
 const branch=JSON.parse(JSON.stringify(role));branch.properties.kind={const:kind};branch.properties.selector.required=required;branch.title=en;branch['x-title-zh']=cn;
 schema.properties.instruments.properties.log.oneOf??=[];schema.properties.instruments.properties.log.oneOf.push(branch);
}
delete schema.properties.instruments.properties.log.type;delete schema.properties.instruments.properties.log.properties;delete schema.properties.instruments.properties.log.required;delete schema.properties.instruments.properties.log.additionalProperties;
schema.properties.editor_revision.readOnly=true;schema.properties.updated_at.readOnly=true;
schema.properties.firmware.properties.path={type:'string',format:'file-path','x-path-base':'config_dir'};
schema.properties.firmware.properties.sha256.readOnly=true;
schema.properties.firmware.properties.sha256.description='Generated by explicit file inspection; preflight independently verifies the actual file hash.';
schema.properties.firmware.properties.sha256['x-description-zh']='由文件检查生成；预检仍会独立复核实际文件哈希。';
schema.allOf=[{if:{properties:{firmware:{properties:{format:{const:'bin'}}}}},then:{properties:{firmware:{required:['base_address'],properties:{base_address:{type:'integer',minimum:0}}}}}}];
const labels={name:'名称',id:'标识',enabled:'启用',order:'排序',tags:'标签',timeout_s:'超时',dut:'DUT 身份',instruments:'仪器绑定',firmware:'固件',reset:'复位',timing:'计时',log_capture:'日志采集',waveform_captures:'波形任务',verdict:'判定',rules:'规则',ai_assist:'AI 辅助',download:'下载（接口预留）',path:'文件',sha256:'SHA-256',format:'格式',target:'目标芯片',base_address:'基地址',method:'方式',channel:'通道',channels:'通道列表',kind:'设备类型',selector:'选择条件',serial:'序列号',resource:'资源地址',port:'串口',baud:'波特率',programmer:'调试探针',scope:'示波器',power:'电源',log:'串口日志',hardware:'硬件复位',dongle_role:'Dongle 角色',power_role:'电源角色',off_time_ms:'断电时长',release_confirmation:'复位释放确认',type:'类型',start_at_s:'开始时间',duration_s:'持续时间',max_lines:'最大行数',encoding:'编码',at_s:'采集时点',source_waveform:'源波形',required:'必需',text:'匹配内容',pattern:'正则',minimum:'下限',maximum:'上限',unit:'单位',capture:'波形任务引用',metric:'指标',limit_ref:'限值引用',mode:'模式',policy:'策略',provider_profile:'服务配置',serial_required:'要求产品序列号',board_revision:'板卡版本',fpga_model:'FPGA 型号',fixture_id:'治具编号',station_id:'工位编号',model:'型号',editor_revision:'编辑版本',updated_at:'更新时间'};
function annotate(node){if(!node||typeof node!=='object')return;for(const[k,v]of Object.entries(node.properties||{})){v.title=k.replaceAll('_',' ');if(labels[k])v['x-title-zh']=labels[k];if(k.endsWith('_s'))v['x-unit']='s';if(k.endsWith('_ms'))v['x-unit']='ms';annotate(v);}for(const key of ['oneOf','anyOf','allOf'])for(const v of node[key]||[])annotate(v);for(const v of Object.values(node.$defs||{}))annotate(v);if(node.items)annotate(node.items);}
schema.$defs.rule.properties.resource_role={type:'string',minLength:1,title:'Instrument role','x-title-zh':'仪器角色'};
schema.$defs.rule=ruleSchema(schema.$defs.rule.properties);
schema.properties.name.default='新测试项';
schema.properties.enabled.default=true;
schema.properties.timeout_s.default=120;
schema.properties.firmware.properties.download.default=false;
annotate(schema);
for(const reset of schema.properties.reset.oneOf)for(const branch of reset.properties.release_confirmation.oneOf)branch.properties.timeout_ms['x-title-zh']='确认超时';
const versionedSchemas=projectSchemas(schema);
fs.writeFileSync(dir+'schemas/fpga-test-case.legacy.schema.json',JSON.stringify(schema,null,2));
fs.writeFileSync(dir+'schemas/fpga-test-case.schema.json',JSON.stringify(versionedSchemas.item,null,2));
fs.writeFileSync(dir+'schemas/project.schema.json',JSON.stringify(versionedSchemas.project,null,2));
const ui={schema_version:'wiparse.config-ui/1',source_view:'read_only',layout:'three_pane',list:{virtualized:true,key:'id',search:['name','id','tags'],actions:['create','duplicate','rename','enable','disable','tag','reorder','archive','restore','import','export']},groups:[['基本信息',['name','id','enabled','order','tags','timeout_s']],['DUT 信息',['dut']],['仪器绑定',['instruments']],['固件',['firmware']],['复位与计时',['reset','timing']],['日志采集',['log_capture']],['波形任务',['waveform_captures']],['判定规则与限值',['verdict.rules']],['AI 辅助',['verdict.ai_assist']]].map(([label,fields])=>({label,fields})),widgets:{'firmware.path':'file','instruments.*':'instrument_selector','tags':'tags','reset':'discriminated_union','waveform_captures':'repeatable_timeline','verdict.rules':'rule_builder'},conditional_fields:[{path:'reset.method',value:'dc_power',show:['reset.power_role','reset.channel','reset.off_time_ms']},{path:'reset.method',value:'wiparse_dongle',show:['reset.dongle_role','reset.hardware']}],validation:{adapter:'fpga-firmware-test',method:'validateCase',cross_fields:['reset','waveform_captures','verdict.rules','timeout_s']},preview:['errors','resources','timeline','estimated_duration','read_only_source']};
const groupNames=['General','DUT identity','Resource bindings','Firmware','Reset and timing','Serial capture','Waveform tasks','Rules and limits','AI advisory'];ui.groups.forEach((g,i)=>{g.label_zh=g.label;g.label=groupNames[i];});
ui.widgets.waveform_captures={type:'timeline',time_field:'at_s',label_field:'id',unit:'s'};
ui.required_widgets=['rule_builder','timeline'];
ui.widgets['firmware.path']={type:'file',hash_field:'firmware.sha256'};
ui.global_actions=['ai_settings'];
ui.widgets.waveform_captures.summary_fields=['id','at_s','channels'];
ui.widgets['verdict.rules']={type:'rule_builder',summary_fields:['metric','minimum','maximum','unit']};
ui.fields={};
const field=(path,options)=>{ui.fields[path]={...(ui.fields[path]||{}),...options};};
const extraLabels={data_bits:'数据位',stop_bits:'停止位',parity:'校验位',flow_control:'流控',manufacturer:'制造商',dongle_serial:'Dongle 序列号',uart_channel:'UART 通道',implementation:'实现状态',verify:'写入校验',minimum_matches:'最少匹配次数',maximum_matches:'最多匹配次数',evidence_ref:'证据文件引用',evidence_refs:'证据引用列表',redaction_profile:'脱敏配置',prompt_template_id:'提示模板',resource_role:'仪器角色',stable_ms:'稳定时长',minimum_v:'最低电压',inputs:'允许输入',dmm:'万用表',load:'电子负载',bridge:'桥接器',schema_version:'配置格式版本',anchor:'计时起点'};
function declareFields(node,prefix=''){for(const[k,v]of Object.entries(node.properties||{})){const p=prefix?`${prefix}.${k}`:k;field(p,{label:v.title||k,label_zh:extraLabels[k]||v['x-title-zh']||k,width:['name','path','pattern','text','sha256'].includes(k)?'long':'normal',...(v.const!==undefined?{hidden:true}:{})});declareFields(v,p);}for(const key of ['oneOf','anyOf','allOf'])for(const v of node[key]||[])declareFields(v,prefix);if(node.items)declareFields(node.items,`${prefix}.*`);}
declareFields(schema);
for(let depth=0;depth<=12;depth++)declareFields(schema.$defs.rule,'verdict.rules.*'+'.rules.*'.repeat(depth));
for(const p of ['schema_version','editor_revision','updated_at'])field(p,{hidden:true,advanced:true});
for(const p of ['order','timeout_s','firmware.base_address','firmware.verify','firmware.implementation','log_capture.max_lines','verdict.ai_assist.provider_profile','verdict.ai_assist.redaction_profile','verdict.ai_assist.prompt_template_id','verdict.ai_assist.inputs'])field(p,{advanced:true});
for(const [i,p]of ['name','id','enabled','tags','timeout_s','order'].entries())field(p,{order:i*10});
field('id',{advanced:true,order:90});
for(const p of ['enabled','order','timeout_s','log_capture.start_at_s','log_capture.duration_s','reset.channel','reset.off_time_ms'])field(p,{width:'short'});
field('dut',{label:'DUT requirements',label_zh:'DUT 身份约束',description:'Expected identity constraints; actual identity is supplied in run parameters.',description_zh:'此处配置期望身份约束；本轮实际身份在运行参数中填写，预检时核对。'});
field('reset.release_confirmation.timeout_ms',{label:'Confirmation timeout',label_zh:'确认超时',width:'short'});
field('timing',{hidden:true,description_zh:'计时起点固定为复位释放确认时刻 T0；已有值仍参与校验。'});
field('reset',{description_zh:'计时起点 T0 固定为复位释放确认时刻。'});
ui.groups[1].label_zh='DUT 身份约束';
field('firmware.download',{description_zh:'当前仅预留下载接口；启用下载会阻断预检。'});
field('verdict.ai_assist',{description_zh:'仅提供建议，不替代规则结论；服务与脱敏策略由工位统一配置。'});
const enumLabels={none:'无',even:'偶校验',odd:'奇校验',instrument_native:'仪器原生格式',csv:'CSV 数据',hex:'HEX',bin:'BIN',frequency_hz:'频率',period_s:'周期',duty_cycle:'占空比',peak_to_peak_v:'峰峰值',minimum_v:'最小电压',maximum_v:'最大电压',mean_v:'平均电压',rise_time_s:'上升时间',fall_time_s:'下降时间',debug_probe:'调试探针',oscilloscope:'示波器',dc_source:'直流电源',serial_port:'COM 串口',wiparse_dongle_uart:'Dongle UART',wiparse_dongle:'WiParse Dongle',dmm:'万用表',load:'电子负载',bridge:'桥接器',voltage_threshold:'电压阈值确认',command_ack:'指令应答确认',command_complete:'指令完成确认'};
function declareEnums(node,prefix=''){
 const choices=node.enum||(node.const!==undefined?[node.const]:[]);
 const localized=Object.fromEntries(choices.filter(v=>typeof v==='string'&&enumLabels[v]).map(v=>[v,enumLabels[v]]));
 if(Object.keys(localized).length)field(prefix,{enum_labels_zh:{...(ui.fields[prefix]?.enum_labels_zh||{}),...localized}});
 for(const[k,v]of Object.entries(node.properties||{}))declareEnums(v,prefix?`${prefix}.${k}`:k);
 for(const key of ['oneOf','anyOf','allOf'])for(const v of node[key]||[])declareEnums(v,prefix);
 if(node.items)declareEnums(node.items,`${prefix}.*`);
}
declareEnums(schema);
for(let depth=0;depth<=12;depth++)declareEnums(schema.$defs.rule,'verdict.rules.*'+'.rules.*'.repeat(depth));
ui.conditional_fields.push(
 {path:'instruments.log.kind',value:'serial_port',show:['instruments.log.selector.port']},
 {path:'instruments.log.kind',value:'wiparse_dongle_uart',show:['instruments.log.selector.dongle_serial','instruments.log.selector.uart_channel']}
);
for(const role of ['programmer','scope','power','log','dmm','load','bridge'])for(const key of ['resource','manufacturer','model','serial','data_bits','stop_bits','parity','flow_control'])field(`instruments.${role}.selector.${key}`,{advanced:true});
for(const role of ['programmer','scope','power'])for(const key of ['serial','resource'])field(`instruments.${role}.selector.${key}`,{advanced:false});
for(let depth=0;depth<=12;depth++)field('verdict.rules.*'+'.rules.*'.repeat(depth)+'.id',{advanced:true,order:90});
field('log_capture.start_at_s',{order:10,label_zh:'开始时间（相对 T0）',description_zh:'T0 为复位释放确认时刻，单位为秒。'});
field('log_capture.duration_s',{order:20,label_zh:'采集时长',description_zh:'从相对 T0 的开始时间起持续采集，单位为秒。'});
// Compatibility-only selector fields remain in stored cases; each branch exposes its applicable fields.
ui.conditional_fields.push({path:'instruments.log.kind',value:'serial_port',hide:['instruments.log.selector.dongle_serial','instruments.log.selector.uart_channel']},{path:'instruments.log.kind',value:'wiparse_dongle_uart',hide:['instruments.log.selector.port']});
ui.required_widgets.push('reference_selector');
const reference=(source_path,value_field='id',label_field='name')=>({type:'reference_selector',source_path,value_field,label_field});
ui.widgets['reset.power_role']=reference('instruments');
ui.widgets['reset.dongle_role']=reference('instruments');
for(let depth=0;depth<=12;depth++){
 const prefix='verdict.rules.*'+'.rules.*'.repeat(depth);
 ui.widgets[`${prefix}.capture`]=reference('waveform_captures');
 ui.widgets[`${prefix}.resource_role`]=reference('instruments');
}
delete ui.widgets['instruments.*'];
ui.widgets['instruments.log.selector.port']={type:'serial_port_selector'};
for(const role of ['programmer','scope','power'])ui.widgets[`instruments.${role}.selector.serial`]={type:'instrument_selector',kind:role==='programmer'?'debug_probe':role==='scope'?'oscilloscope':'dc_source',value_field:'serial'};
ui.execution_badge={parameter:'execution_mode',default:'hardware',values:{simulation:{label:'Simulation · not hardware validation',label_zh:'模拟演示 · 不代表硬件通过',tone:'warning'},hardware:{label:'Hardware mode',label_zh:'真实硬件模式',tone:'normal'}}};
ui.help='Edit by section, save, select cases, then preflight. Simulation uses labelled fixtures; hardware requires an adapter.';
ui.help_zh='按分组编辑并保存，在测试矩阵勾选测试项后预检。查看效果请选择模拟模式；真实硬件需要已验收的适配器。';
const versionedUis=projectUis(ui);
fs.writeFileSync(dir+'schemas/fpga-test-case.ui.json',JSON.stringify(versionedUis.item,null,2));
fs.writeFileSync(dir+'schemas/project.ui.json',JSON.stringify(versionedUis.project,null,2));
const matrixSchema={$schema:schema.$schema,...object({schema_version:{const:'wiparse.test-matrix/1'},plugin_id:{const:'fpga-firmware-test'},revision:{type:'integer',minimum:0},run_id:str,preflight_revision:str,updated_at:{type:'string',format:'date-time'},active_item_id:{type:['string','null']},summary:{type:'object'},items:{type:'array',maxItems:500,items:object({id:str,name:{type:'string',maxLength:128},order:{type:'integer'},tags:{type:'array',maxItems:32,items:{type:'string',maxLength:64}},selected:bool,status:{enum:['pending','running','pass','fail','error','skipped','aborted','inconclusive']},phase:str,progress:{type:['number','null'],minimum:0,maximum:1},attempt:{type:'integer',minimum:1},loop_index:{type:'integer',minimum:0},elapsed_s:num,message:{type:'string',maxLength:256},config_sha256:{type:'string',pattern:'^[a-f0-9]{64}$'},result_ref:{type:['string','null']}},['id','name','tags','selected','status','phase','progress','attempt','elapsed_s','message','result_ref'])}})};
matrixSchema.properties.project_config=object({revision:{type:'integer',minimum:1},sha256:{type:'string',pattern:'^[a-f0-9]{64}$'},schema_hash:{type:'string',pattern:'^[a-f0-9]{64}$'}});
matrixSchema.properties.items.items.properties.attempt_id={type:'string',format:'uuid'};
fs.writeFileSync(dir+'schemas/fpga-matrix-status.schema.json',JSON.stringify(matrixSchema,null,2));
const params=[['execution_mode','enum','hardware'],['simulation_scenario','enum','pass'],['config_dir','path','recipes/fpga-firmware-test/cases'],['selected_ids','string',''],['name_filter','string',''],['tag_filter','string',''],...['dut_serial','board_revision','fpga_model','fixture_id','station_id','operator'].map(k=>[k,'string','']),['loop_count','number',1],['duration_limit_s','number',0],['stop_on_fail','boolean',false],['resume_mode','enum','fresh'],['resume_from','path',''],['ai_global_enable','boolean',false]];
fs.writeFileSync(dir+'plugin.json',JSON.stringify({id:'fpga-firmware-test',name:'FPGA Firmware Test',name_zh:'FPGA固件测试',type:'custom',version:'0.4.0',entry:'index.mjs',config:'station.json',report_name:'FpgaFirmwareTest',description:'Configuration-driven FPGA engineering validation; host adapter required.',capabilities:['configure','preflight','run','stop'],engines:{node:'>=18',wiparse:'>=1.2.1'},sandbox:{permissions:['gui.api','fs.data_root']},ui:{mode:'test_matrix',execution_controls:{control_file:'{test_dir}/fpga-control.json',pause_mode:'between_items'},matrix_status_file:'{test_dir}/matrix.json',max_items:500,config_editor:{schema:'schemas/fpga-test-case.schema.json',ui_schema:'schemas/fpga-test-case.ui.json',repository:'{config_dir}',max_items:500,source_view:'read_only'}},params:params.map(([name,type,value])=>({name,type,label:name,...(value===null?{}:{default:value}),...(['execution_mode','simulation_scenario'].includes(name)?{options:(name==='execution_mode'?['hardware','simulation']:['pass','fail']).map(value=>({value,label:value}))}:{}),...(['selected_ids','name_filter','tag_filter'].includes(name)?{hidden:true}:{}),...(name==='resume_mode'?{options:['fresh','failed','unfinished'].map(value=>({value,label:value}))}:{})})),outputs:['reports','docs','items'].map(id=>({id,label:id,view:id==='reports'?'report':'external',dir:`{artifacts_dir}/${id}`,include:['**/*']}))},null,2));
if(!fs.existsSync(dir+'station.json'))fs.writeFileSync(dir+'station.json',JSON.stringify({schema_version:'wiparse.fpga-station/1',paths:{stop_file:'{test_dir}/fpga.stop',status_file:'{test_dir}/status.json'},late_tolerance_ms:100,minimum_free_bytes:1073741824,safe_power_state:'off',allow_public_ai:false,ai_profiles:{}},null,2));


const manifest=JSON.parse(fs.readFileSync(dir+'plugin.json','utf8'));
manifest.version='0.5.0';manifest.engines.wiparse='>=1.2.2';
manifest.ui.project_config={protocol:'wiparse.project-config/1',schema:'schemas/project.schema.json',ui_schema:'schemas/project.ui.json'};
manifest.ui.result_details={protocol:'wiparse.result-details/1',schema_version:'wiparse.fpga-result/2'};
const paramLabels={resume_from:'历史结果（suite-result.json）',execution_mode:'运行模式',simulation_scenario:'模拟结果',config_dir:'配置目录',dut_serial:'产品序列号',board_revision:'板卡版本',fpga_model:'FPGA型号',fixture_id:'治具编号',station_id:'工位编号',operator:'操作员',loop_count:'循环次数',duration_limit_s:'老化时长上限（秒）',stop_on_fail:'失败即停',resume_mode:'恢复模式',ai_global_enable:'启用 AI 辅助'};
for(const p of manifest.params){if(p.name==='resume_mode')p.options=[{value:'fresh',label:'Fresh run',label_zh:'新建测试'},{value:'failed',label:'Retry failures',label_zh:'重试失败项'},{value:'unfinished',label:'Resume unfinished',label_zh:'继续未完成项'}];if(p.name==='simulation_scenario')p.options=[{value:'pass',label:'Pass fixture',label_zh:'成功场景'},{value:'fail',label:'Failure fixture',label_zh:'失败场景'}];p.label_zh=paramLabels[p.name]||p.name;if(p.name==='execution_mode')p.options=[{value:'hardware',label:'Hardware (adapter required)',label_zh:'真实硬件（需要适配器）'},{value:'simulation',label:'Simulation (no hardware)',label_zh:'模拟演示（不连接仪器）'}];}
fs.writeFileSync(dir+'plugin.json',JSON.stringify(manifest,null,2));

