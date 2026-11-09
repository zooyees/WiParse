export type ExecutionText={id:string;cursor:number;text:string;dropped:number;incomplete:boolean};
export const emptyExecutionText=():ExecutionText=>({id:'',cursor:0,text:'',dropped:0,incomplete:false});
const offset=(value:unknown)=>typeof value==='number'&&Number.isSafeInteger(value)&&value>=0;

/** Byte offsets belong to one host execution, never to a project or another job. */
export function applyExecutionPage(previous:ExecutionText,page:Record<string,unknown>,project:string,plugin:string):ExecutionText{
 if(page.available!==true)return emptyExecutionText();
 if(page.project!==project||page.plugin!==plugin)throw Error('执行日志来源不匹配');
 const id=typeof page.execution_id==='string'?page.execution_id:'';
 if(!id||!offset(page.dropped))throw Error('执行日志身份或保留位置无效');
 const changed=page.changed===true||!!previous.id&&previous.id!==id;
 if(changed)return {id,cursor:page.dropped as number,text:'',dropped:page.dropped as number,incomplete:false};
 if(!offset(page.from)||!offset(page.next)||!offset(page.total)||typeof page.text!=='string')throw Error('执行日志游标无效');
 const from=page.from as number,next=page.next as number,total=page.total as number,dropped=page.dropped as number;
 if(from<previous.cursor||next<from||total<next||from<dropped||next-from>65536||new TextEncoder().encode(page.text).byteLength!==next-from)throw Error('执行日志分片不连续或长度无效');
 const prefix=from>previous.cursor?'[日志保留窗口已前移，部分内容不可用]\n':previous.text;
 let text=(prefix+page.text).slice(-1_048_576);
 // A UTF-16 display window must not start in the middle of an emoji.
 if(text.length&&text.charCodeAt(0)>=0xdc00&&text.charCodeAt(0)<=0xdfff)text=text.slice(1);
 return {id,cursor:next,text,dropped,incomplete:page.incomplete===true};
}
