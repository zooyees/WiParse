import {useEffect,useRef} from 'react';
import {invoke} from '@tauri-apps/api/core';

type Reader=()=>unknown|Promise<unknown>;
const readers=new Map<string,Reader>();
/** Register a reader; no data is gathered or sent until the user requests it. */
export function useWorkspaceSummary(tool:string,reader:Reader){
  const current=useRef(reader);current.current=reader;
  useEffect(()=>{const read=()=>current.current();readers.set(tool,read);return()=>{if(readers.get(tool)===read)readers.delete(tool);};},[tool]);
}
export async function readWorkspaceSummary(tool:string):Promise<string>{
  const reader=readers.get(tool);const value=reader?await reader():await invoke('ai_page_summary',{page:tool});
  const text=JSON.stringify({tool,captured_at:new Date().toISOString(),snapshot:value},null,2);
  if(new TextEncoder().encode(text).length>32768)throw Error('页面摘要超过 32 KiB，请选择较小的数据范围或手动填写上下文。');
  return text;
}
