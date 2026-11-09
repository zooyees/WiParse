import {object} from './model.ts';

type ReadPage=(params:Record<string,unknown>)=>Promise<unknown>;
type SearchResult={row:number|null;dropped:number}|null;

/** Page-level shortcuts must also work when focus has returned to the WebView document. */
export function serialLogShortcut(key:string,control:boolean,shift:boolean,alt:boolean,repeat:boolean):'goto'|'find'|'next'|'previous'|'escape'|null{
  if(alt||repeat)return null;
  if(control&&!shift&&key.toLowerCase()==='g')return 'goto';
  if(control&&!shift&&key.toLowerCase()==='f')return 'find';
  if(!control&&key==='F3')return shift?'previous':'next';
  if(!control&&!shift&&key==='Escape')return 'escape';
  return null;
}

export function resolveLogTab(entries:unknown[],tab:number|null,view:number|null,active:number):Record<string,unknown>|undefined {
  const rows=entries.map(object);
  return view!==null?rows.find(row=>row.view_id===view):rows.find(row=>row.tab_id===(tab??active));
}

/** Search one retained snapshot in either direction, wrapping at its bounds. */
export async function searchSerialHistory(read:ReadPage,tabId:number,viewId:number,query:string,after:number|null,cancelled:()=>boolean,progress:(cursor:number,end:number)=>void,pause:()=>Promise<void>,caseSensitive=false,direction:'next'|'previous'='next'):Promise<SearchResult>{
  if(!query)return {row:null,dropped:0};
  const snapshot=await serialSnapshot(read,tabId,viewId,cancelled);if(!snapshot)return null;
  const {beginning,end,pageAt}=snapshot;
  if(after!==null&&!Number.isSafeInteger(after))throw Error('日志查找位置无效');
  const pivot=after===null?null:Math.max(beginning,Math.min(end,after+(direction==='next'?1:0)));
  const intervals=pivot===null?[[beginning,end]]:direction==='next'?[[pivot,end],[beginning,pivot]]:[[beginning,pivot],[pivot,end]];
  const needle=caseSensitive?query:query.toLocaleLowerCase();let scanned=0;
  for(const [from,to] of intervals){
    let edge=direction==='next'?from:to;
    while(direction==='next'?edge<to:edge>from){
      let cursor=direction==='next'?edge:Math.max(from,edge-500);const stop=direction==='next'?to:edge;let last:number|null=null;
      while(cursor<stop){
        const page=await pageAt(cursor,stop);if(!page)return null;
        for(let index=0;index<page.rows.length;index++){
          const line=caseSensitive?page.rows[index]:page.rows[index].toLocaleLowerCase();
          if(line.includes(needle)){if(direction==='next')return {row:cursor+index,dropped:beginning};last=cursor+index;}
        }
        scanned+=page.next-cursor;cursor=page.next;progress(Math.min(scanned,end-beginning),end-beginning);
        await pause();if(cancelled())return null;
        if(direction==='next')break;
      }
      if(last!==null)return {row:last,dropped:beginning};
      edge=direction==='next'?cursor:Math.max(from,edge-500);
    }
  }
  return {row:null,dropped:beginning};
}

export function retainedRowInput(value:string,dropped:number,total:number):number|null{
  if(!/^[1-9][0-9]*$/.test(value.trim()))return null;
  const number=Number(value.trim());if(!Number.isSafeInteger(number))return null;
  const row=number-1;
  return Number.isSafeInteger(row)&&row>=dropped&&row<total?row:null;
}

async function serialSnapshot(read:ReadPage,tabId:number,viewId:number,cancelled:()=>boolean){
  const initial=object(await read({tab_id:tabId,view_id:viewId,from_row:0,limit:1,max_bytes:1_048_576}));
  if(cancelled())return null;
  const beginning=initial.dropped,end=initial.total,epoch=initial.content_epoch;
  if(initial.view_id!==viewId||typeof epoch!=='number'||!Number.isSafeInteger(epoch)||epoch<0||typeof beginning!=='number'||typeof end!=='number'||!Number.isSafeInteger(beginning)||!Number.isSafeInteger(end)||beginning<0||end<beginning)throw Error('日志范围无效');
  const pageAt=async(cursor:number,stop:number)=>{
    const page=object(await read({tab_id:tabId,view_id:viewId,content_epoch:epoch,from_row:cursor,limit:Math.min(500,stop-cursor),max_bytes:1_048_576}));
    if(cancelled())return null;
    if(page.view_id!==viewId||page.content_epoch!==epoch||page.dropped!==beginning||typeof page.total!=='number'||!Number.isSafeInteger(page.total)||page.total<end||page.truncated===true)throw Error('日志内容已变化，请重新查找');
    const rows=Array.isArray(page.lines)?page.lines:[],next=page.next_row;
    if(!rows.every(line=>typeof line==='string')||typeof next!=='number'||!Number.isSafeInteger(next)||next<=cursor||next>stop||next-cursor!==rows.length)throw Error('日志页面不完整');
    return {rows:rows as string[],next};
  };
  return {beginning,end,epoch,pageAt};
}

export type SerialMatches={tabId:number;viewId:number;epoch:number;dropped:number;total:number;count:number;rows:{row:number;preview:string}[];limited:boolean};
/** Scan every retained row once; result previews are bounded independently of log capacity. */
export async function collectSerialMatches(read:ReadPage,tabId:number,viewId:number,query:string,cancelled:()=>boolean,progress:(cursor:number,total:number)=>void,pause:()=>Promise<void>,caseSensitive=false):Promise<SerialMatches|null>{
  if(!query)return null;
  const snapshot=await serialSnapshot(read,tabId,viewId,cancelled);if(!snapshot)return null;
  const {beginning,end,epoch,pageAt}=snapshot,needle=caseSensitive?query:query.toLocaleLowerCase();
  const rows:SerialMatches['rows']=[];let cursor=beginning,count=0;
  while(cursor<end){
    const page=await pageAt(cursor,end);if(!page)return null;
    for(let index=0;index<page.rows.length;index++){
      const text=page.rows[index];if((caseSensitive?text:text.toLocaleLowerCase()).includes(needle)){
        count++;if(rows.length<1000)rows.push({row:cursor+index,preview:text.slice(0,240)});
      }
    }
    cursor=page.next;progress(cursor-beginning,end-beginning);await pause();if(cancelled())return null;
  }
  return {tabId,viewId,epoch,dropped:beginning,total:end,count,rows,limited:count>rows.length};
}
