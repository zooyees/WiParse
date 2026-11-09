import {object,string} from './model.ts';
export type WorkbenchPage='tests'|'instruments'|'serial'|'wave'|'data'|'report'|'calculator'|'settings';
export type Navigation={owner:number;revision:number;page:WorkbenchPage};
const destinations:Record<string,WorkbenchPage>={serial:'serial',calculator:'calculator',instruments:'instruments',waveform:'wave',data_analysis:'data',test_report:'report',test_tool:'tests'};
export function navigationRequest(raw:unknown):Navigation|null {
  const value=object(raw),page=destinations[string(value.tab)];
  if(value.state!=='requested'||!page||!Number.isSafeInteger(value.owner)||Number(value.owner)<=0||Number(value.owner)>0xffffffff||!Number.isSafeInteger(value.revision)||Number(value.revision)<=0)return null;
  return {owner:Number(value.owner),revision:Number(value.revision),page};
}
