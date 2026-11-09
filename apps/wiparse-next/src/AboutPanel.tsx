import {useEffect,useState} from 'react';
import {invoke} from '@tauri-apps/api/core';
import {LocalizedLabel,localizeLabel} from './localization';

type Info={version:string;shell_version:string;platform:string;architecture:string};
export function AboutPanel(){
  const [info,setInfo]=useState<Info|null>(null),[error,setError]=useState('');
  useEffect(()=>{let active=true;invoke<Info>('application_info').then(value=>{if(active)setInfo(value);}).catch(reason=>{if(active)setError(String(reason));});return()=>{active=false;};},[]);
  return <section className="settings-content about-panel"><div className="settings-title"><div><strong>WiParse</strong><small><LocalizedLabel text="仪器控制、串口、波形与自动化测试工作台"/></small></div></div><dl className="about-facts"><div><dt><LocalizedLabel text="软件版本"/></dt><dd>{info?.version??'—'}</dd></div><div><dt><LocalizedLabel text="新界面版本"/></dt><dd>{info?.shell_version??'—'}</dd></div><div><dt><LocalizedLabel text="运行平台"/></dt><dd>{info?`${info.platform} · ${info.architecture}`:'—'}</dd></div></dl><h3><LocalizedLabel text="运行环境"/></h3><p><LocalizedLabel text="新界面需要 WebView2；插件与 MCP 需要 Node.js 24。实际仪器需要对应驱动及 VISA。"/></p><h3><LocalizedLabel text="版本更新"/></h3><p><LocalizedLabel text="当前为新界面预览版。请保留完整运行目录，使用匹配的新界面更新包升级。"/></p><p className="detail-help"><LocalizedLabel text="旧版自动更新包不能保证包含新界面和配套宿主，请在设置的“运行包更新”中检查、导入和校验完整运行包，确认后关闭安装；失败可回滚。"/></p>{error&&<p role="alert">{localizeLabel('版本信息读取失败：')}{error}</p>}</section>;
}
