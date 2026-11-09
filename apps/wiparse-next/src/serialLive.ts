import {object} from './model.ts';

export const SERIAL_FRAME_MS=33;
/** Only explicit reading gestures pause follow. Browser scroll events also come
 * from layout, font changes and replacing a bounded rolling page. */
export function pausesSerialFollow(input:{kind:'wheel';deltaY:number;control:boolean;meta:boolean}|{kind:'key';key:string}|{kind:'pointer';button:number;x:number;right:number;scrollbar:number}){
 if(input.kind==='wheel')return input.deltaY<0&&!input.control&&!input.meta;
 if(input.kind==='key')return ['ArrowUp','PageUp','Home'].includes(input.key);
 return input.button===0&&input.scrollbar>0&&input.x>=input.right-input.scrollbar;
}

/** Resolve absolute tail coordinates after FIFO eviction, without changing the
 * host retention limit or treating a historical page as a live one. */
export async function readSerialPage(read:(params:Record<string,unknown>)=>Promise<unknown>,tab:number,view:number|undefined,count:number,offset:number|null,active:()=>boolean){
 const identity={tab_id:tab,...(view===undefined?{}:{view_id:view})};
 let page=object(await read({...identity,from_row:offset??Math.max(0,count-500),limit:500}));
 if(!active())return null;
 const end=typeof page.total==='number'?page.total:0;
 if(offset===null&&end>500&&page.next_row!==end){
  page=object(await read({...identity,from_row:end-500,limit:500}));
  if(!active())return null;
 }
 return page;
}
