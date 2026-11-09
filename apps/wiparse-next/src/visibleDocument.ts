/** Keep hidden values attached to their original location when arrays move. */
export function bindVisibleOrigins(value:unknown,path:string[]=[]):unknown {
  if(value==='[LOCAL_PATH]'||value==='[REDACTED]')return `[${value.slice(1,-1)}@${encodeURIComponent(JSON.stringify(path))}]`;
  if(Array.isArray(value))return value.map((item,index)=>bindVisibleOrigins(item,[...path,String(index)]));
  if(value&&typeof value==='object')return Object.fromEntries(Object.entries(value).map(([key,item])=>[key,bindVisibleOrigins(item,[...path,key])]));
  return value;
}
export function isPreservedValue(value:unknown):value is string {
  return typeof value==='string'&&/^\[(?:LOCAL_PATH|REDACTED)(?:@[^\r\n]+)?\]$/.test(value);
}
