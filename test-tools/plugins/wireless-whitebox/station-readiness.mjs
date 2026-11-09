import {createHttpClient} from '../../lib/wiparse-sdk.mjs';

export function serialPortsFromReply(reply) {
  if (reply?.ok === false) throw new Error('serial request failed');
  const ports=Array.isArray(reply?.data)?reply.data:reply?.data?.ports??reply?.ports;
  if(!Array.isArray(ports))throw new Error('invalid serial response');
  return ports;
}

// Read-only inspection. A matching name is never treated as proof of device
// capabilities, armed acquisition or safe physical output state.
export async function inspectStation(ctx) {
  const station=ctx.projectConfig?.value?.station || {};
  const checks=[];
  const add=(code,ok,message)=>checks.push({code,ok,message});
  const invoke=ctx.invoke ?? createHttpClient({url:ctx.url,log:ctx.log}).invoke;
  let devices=[],ports=[];
  try {
    const r=await invoke('instrument.list',{},5000);devices=r?.data?.devices ?? r?.devices ?? [];
    if(!Array.isArray(devices))throw new Error('invalid instrument response');
    add('INSTRUMENT.LIST',true,'已读取当前连接的仪器');
  } catch(e) {add('INSTRUMENT.LIST',false,`无法读取已连接仪器：${e.message}`);}
  try {
    const r=await invoke('serial.ports',{},5000);ports=serialPortsFromReply(r);
    add('SERIAL.LIST',true,'已读取串口列表');
  } catch(e) {add('SERIAL.LIST',false,`无法读取串口：${e.message}`);}
  for(const [role,label,resource,kinds] of [
    ['SCOPE','示波器',station.scope_resource,['oscilloscope']],
    ['POWER','电源',station.power_resource,['dcsource']],
  ]) {
    const configured=String(resource||'').trim();
    const matches=devices.filter(d=>String(d.resource||'')===configured && configured);
    const model=String(matches[0]?.identity?.model??matches[0]?.model??'').toUpperCase();
    const kind=String(matches[0]?.kind||'').toLowerCase().replace(/[^a-z]/g,'');
    const kindMatches=kinds.includes(kind);
    add(`${role}.BINDING`,matches.length===1&&kindMatches,
      !configured?`${label} VISA 地址未填写`:matches.length!==1?`${label}未唯一匹配已连接的 VISA 地址`:
      !kindMatches?`${label}设备类型不匹配：${kind||'未知'}`:`${label}已匹配 ${model||'型号未知'}；采集和控制能力需实测`);
  }
  const port=String(station.log_source||'').trim();
  add('UART.BINDING',!!port&&ports.some(p=>String(p.device??p.port??'').toUpperCase()===port.toUpperCase()),port?'日志串口需与当前串口列表匹配':'日志串口未填写');
  const loadResource=String(station.load_resource||'').trim();
  const loads=devices.filter(d=>loadResource&&String(d.resource||'')===loadResource&&String(d.kind||'').toLowerCase().replace(/[^a-z]/g,'')==='electronicload');
  add('LOAD.BINDING',loads.length===1,!loadResource?'电子负载 VISA 地址未填写':loads.length!==1?'电子负载未唯一匹配已连接设备':'电子负载已匹配；动态能力和量程需实测');
  const bridgeResource=String(station.rx_i2c_interface||'').trim();
  const bridges=devices.filter(d=>bridgeResource&&String(d.resource||'')===bridgeResource&&['usbbridge','bridge'].includes(String(d.kind||'').toLowerCase().replace(/[^a-z]/g,'')));
  add('RX.FT4222_BINDING',bridges.length===1,!bridgeResource?'MT5788 的 FT4222 设备未选择':bridges.length!==1?'FT4222 未唯一匹配已连接设备':'FT4222 已匹配；I2C 事务与设备能力需实测');
  add('RX.I2C_CONTRACT',false,'已依据 MaxicCTP 实现 MT5788 寄存器与 FT4222 命令适配，尚未完成设备独占和实机验收');
  add('TEST.LIMITS',false,'具体产品测试限值和工况尚待提供');
  add('CAPTURE.RAW_TIMING',false,'原始串口采集、示波器 armed 反馈和同步时序尚未验收');
  add('STATION.SAFE_STOP',false,'工位输出控制及安全停止反馈尚未验收');
  add('WIRELESS.LIVE_BACKEND_NOT_VALIDATED',false,'无线充电实物执行器尚未接通');
  return {ok:false,checks};
}
