"""Rebuild versioned SIMULATION data. Never generates approved product limits."""
import json
from pathlib import Path
ROOT = Path(__file__).resolve().parents[1]
V = 'wiparse.wireless-plugin/1'
def save(name, value):
    p = ROOT / name
    p.parent.mkdir(parents=True, exist_ok=True)
    p.write_text(json.dumps({'schema_version': V, 'version': '0.1.0', **value}, ensure_ascii=False, indent=2) + '\n', encoding='utf-8')

save('plugin.json', {
 'id':'wireless-charge-validation','name':'Wireless Charge Validation','name_zh':'无线充电自动检验',
 'version':'0.1.0','type':'custom','entry':'index.mjs','config':'station.json','report_name':'WirelessCharge','report_name_zh':'无线充电检验',
 'description':'Contract S1: dry-run/replay, product rules and traceable coverage. Live stimulus blocked until SW/DG safety integration is validated.',
 'capabilities':['preflight','run','stop'],'engines':{'node':'>=18','wiparse':'>=1.1.29'},
 'outputs':[{'id':'reports','label':'Validation evidence','label_zh':'检验证据','view':'report','dir':'{artifacts_dir}/reports','include':['*.md','*.json','*.jsonl']}],
 'params':[
  {'name':'product','type':'string','default':'reference-tx','label':'Product','label_zh':'产品配置'},
  {'name':'suite','type':'enum','default':'smoke','label':'Suite','label_zh':'测试档','options':[{'value':x,'label':x} for x in ['smoke','regression','interop','soak','production']]},
  {'name':'mode','type':'enum','default':'dry-run','label':'Mode','label_zh':'运行模式','options':[{'value':x,'label':x} for x in ['dry-run','replay','live']]},
  {'name':'fixture','type':'enum','default':'gold','label':'Replay database','label_zh':'回放数据库','options':[{'value':x,'label':x} for x in ['gold','bad','missing','quality']]},
  {'name':'dut_serial','type':'string','default':'','label':'DUT serial','label_zh':'DUT序列号'},
  {'name':'dut_firmware','type':'string','default':'','label':'DUT firmware','label_zh':'DUT固件'},
  {'name':'service_root','type':'path','default':'','label':'Service data root','label_zh':'自动服务数据目录','advanced':True},
  {'name':'file_prefix','type':'string','default':'WirelessCharge','label':'Report name','label_zh':'报告名称'}]})
save('station.json', {'station_id':'UNCOMMISSIONED','gui':{'api':'http://127.0.0.1:7878','min_version':'1.1.29'},'bindings':{},'protection':{'verified':False,'stop_sequence_approved':False,'stop_sequence':[]},'g4_approved':False,'timing':{'event_p99_target_ms':250,'health_target_ms':100,'lost_alarm_target_ms':500,'measured':False},'paths':{}})
save('products/reference-tx/profile.json', {'id':'reference-tx','simulation_only':True,'topology':'TX-reference-not-approved','protocols':['USBPD','SCP','UFCS','Qi'],'vin_mapping':'external-input','vbrg_mapping':'external-bridge','power_w':None,'dut_fields':{},'approved':False})
save('products/reference-tx/states.json', {'layers':{'input':['OFF','NEGOTIATING','READY','FAULT'],'tx':['IDLE','PING','IDENTIFY','PT','EPT','FAULT'],'battery':['UNKNOWN','CC','CV','DONE']},'transitions':{'input':{'OFF':['NEGOTIATING'],'NEGOTIATING':['READY','FAULT'],'READY':['OFF','FAULT'],'FAULT':['OFF']},'tx':{'IDLE':['PING'],'PING':['IDENTIFY','IDLE','FAULT'],'IDENTIFY':['PT','FAULT'],'PT':['EPT','FAULT'],'EPT':['IDLE'],'FAULT':['IDLE']},'battery':{'UNKNOWN':['CC'],'CC':['CV'],'CV':['DONE']}}})
limits = {
 'voltage':{'unit':'V','minimum':4.5,'maximum':5.5},
 'vbrg':{'unit':'V','minimum':8,'maximum':12},
 'frequency':{'unit':'Hz','minimum':90000,'maximum':150000},
 'deadtime':{'unit':'s','minimum':0.0000001,'maximum':0.000001},
 'ratio':{'unit':'1','minimum':0,'maximum':0.05},
 'power':{'unit':'W','minimum':3,'maximum':10},
 'residual':{'unit':'V','minimum':0,'maximum':0.5},
 'latency':{'unit':'s','minimum':0,'maximum':0.25},
 'integrity':{'unit':'1','minimum':1,'maximum':1},
 'temperature':{'unit':'°C','minimum':10,'maximum':45}}
for item in limits.values(): item.update({'source':'SIMULATOR ONLY; replace with approved product clause','state_window_required':True,'guard_band':'measurement uncertainty','approval_authority':'simulation database'})
save('products/reference-tx/limits.json', {'approved':True,'simulation_only':True,'approval_scope':'simulator-regression-only','items':limits})
save('catalogs/adapters.json', {'objects':[{'id':p.lower(),'type':'protocol-emulator-placeholder','protocol':p,'verified':False,'real_adapter':False,'serial':None} for p in ['USBPD','SCP','UFCS','FixedDC']]})
save('catalogs/receivers.json', {'objects':[{'id':'controlled-rx','type':'controlled-receiver','verified':False},{'id':'phone','type':'real-phone','soc':None,'temperature_c':None,'verified':False}]})
save('catalogs/cables.json', {'objects':[{'id':'reference-cable','resistance_ohm':None,'rating_a':None,'verified':False}]})
cases=[]
def case(id, goal, sys, pl, metric='integrity', limit='integrity', state='PT', channel='external', kind='scalar', mandatory=False, layer=None):
    check={'id':id+'.01','kind':kind,'required':True,'metric_id':metric,'limits_ref':limit,'state':state,'channel':channel,'window_s':[0,0.01]}
    if kind=='state': check.update({'layer':layer,'expected_final':'PT' if layer=='tx' else 'READY','minimum_power_w':3,'allowed_ept_reasons':['charge_complete','approved_removal']})
    if kind=='communication':check.update({'layer':layer,'maximum_error_ratio':0.01,'maximum_timeouts':0,'denominator':'independent expected frames; never observed frames alone'})
    cases.append({'id':id,'goal':goal,'sys':sys,'pl':pl,'sw':['SW-08','SW-09'],'dg':['DG-02','DG-04'],'mandatory':mandatory,'initial_state':'SAFE_READY','risk':'No live action until product/station risk approval','roles':['scope','dongle','source','receiver','protection'],'required_capabilities':['device.lease.acquire','capture.arm','capture.status','dongle.capture.start','station.safe_stop','stimulus.apply','check.evaluate'],'deadline_ms':5000,'capture_plan':{'raw':True,'pretrigger_s':0.01,'posttrigger_s':0.1,'same_capture_id_required':True,'timing_budget_approved':False},'steps':[{'id':'ready','condition':'Dongle ready AND scope armed AND protection healthy','deadline_ms':5000},{'id':'stimulus','condition':'lease/fencing/unique command; approved BBM and feedback','deadline_ms':1000},{'id':'observe','condition':'capture complete AND quality validated','deadline_ms':5000}],'checks':[check],'cleanup':'station approved topology-specific stop sequence and residual feedback','recovery':'unknown output: isolate; no retry; new attempt after preflight'})
for i, goal in enumerate(['输入协议合同/物理VI/持续充电','真实适配器互操作','线缆压降与能力','多口共享与合同变化','回退到批准输入','协商超时与恢复','冷启动/热插拔','供电掉电/恢复','历史适配器故障回归'],1):
    case(f'ADP{i:02}',goal,['SYS-01'],['PL-02','PL-05','PL-09'],mandatory=i in [1,9])
case('VIN01','VIN物理值与请求反馈分离',['SYS-02'],['PL-06'],'vin_mean','voltage',mandatory=True,channel='VIN')
case('VBRG01','VBRG启动稳态与控制',['SYS-02'],['PL-06'],'vbrg_mean','vbrg',channel='VBRG')
case('VIN02','过冲/纹波独立极值',['SYS-02'],['PL-06'],'vin_peak','voltage',channel='VIN')
case('VIN03','关断残压及重复启动',['SYS-02','SYS-07'],['PL-06','PL-04'],'residual_voltage','residual','SAFE_READY','VIN')
case('PWM01','PING burst工作频率(排除idle)',['SYS-03'],['PL-07'],'frequency_median','frequency','PING','PWM',mandatory=True)
case('PWM02','PT调频轨迹与占空比',['SYS-03'],['PL-07'],'frequency_median','frequency','PT','PWM')
for direction in ['HL','LH']:
    for bridge in ['A','B']:case('DT'+bridge+direction,'实际VGS双向最坏死区 '+bridge+direction,['SYS-03'],['PL-07'],'deadtime_min_'+direction,'deadtime','PT','VGS-'+bridge)
case('STATE01','TX日志与物理功率一致性',['SYS-04'],['PL-03','PL-06'],kind='state',layer='tx',mandatory=True)
case('INPUT01','输入层状态与供电过程',['SYS-04','SYS-01'],['PL-03'],kind='state',layer='input')
case('BAT01','RX电池CC/CV可观测性',['SYS-04'],['PL-03','PL-09'])
for layer in ['input','qi']:case('COMM-'+layer.upper(),'通信顺序/参考分母/超时 '+layer,['SYS-05'],['PL-08','PL-09'],kind='communication',layer=layer,mandatory=True)
for i,goal in enumerate(['同窗校准功率/效率','手机SOC及充电终止','热稳态/降额','偏位与耦合','正式FOD夹具','移除/重放','保护/复位','长稳/周期','冻结条件A/B'],1):case(f'CHG{i:02}',goal,['SYS-04','SYS-09'],['PL-09','PL-12'],mandatory=i==7)
case('SAFE01','保护先行与异常捕获时延',['SYS-06'],['PL-04','PL-10'],'event_visibility_latency','latency',mandatory=True)
case('EXEC01','独占/unknown/取消/有限恢复',['SYS-07'],['PL-01','PL-04','PL-10'],mandatory=True)
case('FAULT01','首个异常、证据及隔离处置',['SYS-08'],['PL-10','PL-11'],mandatory=True)
case('TRACE01','金坏样/量测系统/版本/生产追溯',['SYS-10'],['PL-11','PL-12'],mandatory=True)
save('scenarios/cases.json', {'cases':cases})
mandatory=[c['id'] for c in cases if c['mandatory']]
for suite in ['smoke','regression','interop','soak','production']:
    save('suites/'+suite+'.json', {'id':suite,'industrial':suite!='smoke','case_ids':mandatory if suite=='smoke' else [c['id'] for c in cases], 'repetitions':1,'stop_policy':'quality missing prohibits next stimulus; protection isolates','historical_fault_case_ids':['ADP09','FAULT01'],'coverage_complete_required':suite!='smoke','live_enabled':False})
gold={}
for c in cases:
    s=c['checks'][0]
    if s['kind']=='scalar':
        l=limits[s['limits_ref']]; v=(l['minimum']+l['maximum'])/2
        o={'state':s['state'],'measurement':{'schema_version':'wiparse.automation/1','metric_id':s['metric_id'],'channel':s['channel'],'value':v,'unit':l['unit'],'sample_count':100,'window':[0,0.01],'uncertainty':0,'quality':{'issues':[]},'algorithm_version':'SIMULATION-reference/1','evidence_refs':['simulation://'+c['id']]}}
    elif s['kind']=='state':
        states=['IDLE','PING','IDENTIFY','PT'] if s['layer']=='tx' else ['OFF','NEGOTIATING','READY']
        o={'events':[{'time_s':i*0.01,s['layer']:state,'physical_power_w':5,'quality':{'issues':[]},'evidence_refs':['simulation://'+c['id']]} for i,state in enumerate(states)]}
    else:o={'layer':s['layer'],'expected_count':100,'received_count':100,'error_count':0,'timeout_count':0,'complete':True,'reference_verified':True,'sequence_valid':True,'quality':{'issues':[]},'evidence_refs':['simulation://'+c['id']]}
    gold[c['id']]={s['id']:o}
save('tests/fixtures/gold.json', {'simulation_only':True,'cases':gold})
bad=json.loads(json.dumps(gold))
for c in cases:
    s=c['checks'][0];o=bad[c['id']][s['id']]
    if s['kind']=='scalar':o['measurement']['value']=limits[s['limits_ref']]['maximum']+max(1,abs(limits[s['limits_ref']]['maximum']))
    elif s['kind']=='state':o['events']=o['events'][-1:];o['events'][0]['physical_power_w']=0;s['layer']=='tx' or o['events'][0].update({'input':'OFF'})
    else:o['received_count']=50
save('tests/fixtures/bad.json', {'simulation_only':True,'cases':bad})
save('tests/fixtures/missing.json', {'simulation_only':True,'cases':{}})
quality=json.loads(json.dumps(gold))
for c in cases:
    s=c['checks'][0];o=quality[c['id']][s['id']]
    if s['kind']=='scalar':o['measurement']['quality']['issues']=['DG.OVERFLOW','TIME.BOOT_CHANGED']
    elif s['kind']=='state':o['events'][0]['quality']['issues']=['STREAM.SEQ_GAP']
    else:o['quality']['issues']=['CAPTURE.BLIND_WINDOW']
save('tests/fixtures/quality.json', {'simulation_only':True,'cases':quality})
