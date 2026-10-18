export function projectSchemas(legacy){
 const item=structuredClone(legacy);
 item.properties.schema_version.const='wiparse.fpga-test-case/2';
 delete item.properties.dut;delete item.properties.instruments;
 item.required=item.required.filter(k=>!['dut','instruments'].includes(k));
 item.properties.log_capture.properties.resource_role={type:'string',minLength:1,title:'Resource role','x-title-zh':'日志角色'};
 item.properties.log_capture.allOf=[{if:{properties:{enabled:{const:true}},required:['enabled']},then:{required:['resource_role']}}];
 const instruments=structuredClone(legacy.properties.instruments);
 instruments.additionalProperties=structuredClone(legacy.properties.instruments.properties.programmer);
 delete instruments.additionalProperties.title;delete instruments.additionalProperties['x-title-zh'];
 instruments.properties.log_com=structuredClone(instruments.properties.log.oneOf.find(b=>b.properties.kind.const==='serial_port'));
 instruments.properties.log_dongle=structuredClone(instruments.properties.log.oneOf.find(b=>b.properties.kind.const==='wiparse_dongle_uart'));
 instruments.properties.log_com['x-title-zh']='COM 日志角色';instruments.properties.log_dongle['x-title-zh']='Dongle UART 日志角色';
 const project={$schema:legacy.$schema,title:'FPGA project configuration','x-title-zh':'FPGA 工程共享配置',type:'object',properties:{schema_version:{const:'wiparse.fpga-project/1'},name:{type:'string',minLength:1,maxLength:128,title:'Project name','x-title-zh':'工程名称'},description:{type:'string',maxLength:2048,title:'Description','x-title-zh':'工程说明'},dut:structuredClone(legacy.properties.dut),instruments},required:['schema_version','name','dut','instruments'],additionalProperties:false};
 return {item,project};
}
export function projectUis(legacy){
 const item=structuredClone(legacy),sharedFields=p=>p==='dut'||p.startsWith('dut.')||p==='instruments'||p.startsWith('instruments.');
 item.groups=item.groups.slice(3);
 item.item_properties=['name','id','enabled','order','tags','timeout_s'];
 item.fields=Object.fromEntries(Object.entries(item.fields).filter(([p])=>!sharedFields(p)));
 item.widgets=Object.fromEntries(Object.entries(item.widgets).filter(([p])=>!p.startsWith('instruments.')));
 item.conditional_fields=item.conditional_fields.filter(c=>!c.path.startsWith('instruments.'));
 for(const[p,widget]of Object.entries(item.widgets))if(widget?.type==='reference_selector'&&widget.source_path==='instruments')widget.source_scope='project_config';
 item.widgets['log_capture.resource_role']={type:'reference_selector',source_scope:'project_config',source_path:'instruments',value_field:'id',label_field:'name'};
 item.fields['log_capture.resource_role']={label:'Log role',label_zh:'日志角色',order:0,description_zh:'引用工程中已保存的COM或Dongle UART角色；绑定只在工程设置中修改。'};
 item.help_zh='工程名称、DUT约束及仪器绑定在工程设置中统一维护。测试名称等由项属性编辑；日志、复位和规则仅引用共享角色。';
 const fields=Object.fromEntries(Object.entries(legacy.fields).filter(([p])=>sharedFields(p)));
 fields.schema_version={hidden:true};fields.name={label:'Project name',label_zh:'工程名称',width:'long',order:0};fields.description={label:'Description',label_zh:'工程说明',width:'long',order:10};
 for(const role of ['log_com','log_dongle'])for(const[p,v]of Object.entries(legacy.fields))if(p.startsWith('instruments.log.'))fields[p.replace('instruments.log.',`instruments.${role}.`)]=structuredClone(v);
 for(const role of ['log_com','log_dongle'])fields[`instruments.${role}`]={label:role==='log_com'?'COM log role':'Dongle UART log role',label_zh:role==='log_com'?'COM 日志角色':'Dongle UART 日志角色'};
 for(const key of ['dongle_serial','uart_channel'])fields[`instruments.log_com.selector.${key}`]={hidden:true};fields['instruments.log_dongle.selector.port']={hidden:true};
 const widgets=Object.fromEntries(Object.entries(legacy.widgets).filter(([p])=>p.startsWith('instruments.')));
 widgets['instruments.log_com.selector.port']={type:'serial_port_selector'};
 widgets['instruments.log_dongle.selector.dongle_serial']={type:'instrument_selector',kind:'wiparse_dongle_uart',value_field:'dongle_serial'};
 const project={schema_version:'wiparse.config-ui/1',source_view:'read_only',groups:[{label:'Project',label_zh:'工程基本信息',fields:['name','description']},{label:'DUT requirements',label_zh:'DUT 身份约束',fields:['dut']},{label:'Resource bindings',label_zh:'仪器绑定',fields:['instruments']}],fields,widgets,conditional_fields:legacy.conditional_fields.filter(c=>c.path.startsWith('instruments.')),help_zh:'当前测试工程的共享设置，按project/test/plugin隔离；保存后才可预检。实际DUT身份在本轮参数填写，不由共享默认值覆盖。'};
 return {item,project};
}
