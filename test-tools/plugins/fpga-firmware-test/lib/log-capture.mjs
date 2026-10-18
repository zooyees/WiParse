import fs from 'node:fs';
import path from 'node:path';
import { fail } from './common.mjs';
export function preserveLog(log, dir, config) {
  const folder=path.join(dir,'logs'); fs.mkdirSync(folder,{recursive:true});
  if(!log || !Array.isArray(log.lines) || typeof log.raw_base64!=='string') throw fail('LOG.RAW_EVIDENCE_REQUIRED');
  fs.writeFileSync(path.join(folder,'serial.raw'),Buffer.from(log.raw_base64,'base64'));
  const raw=Buffer.from(log.raw_base64,'base64');
  let decoded=true,text='';
  try {text=new TextDecoder('utf-8',{fatal:true}).decode(raw);} catch {decoded=false;}
  const fd=fs.openSync(path.join(folder,'serial.jsonl'),'w');
  try {for(let i=0;i<log.lines.length;i+=1024) fs.writeSync(fd,log.lines.slice(i,i+1024).map(line=>JSON.stringify(line)+'\n').join(''));fs.fsyncSync(fd);} finally {fs.closeSync(fd);}
  fs.writeFileSync(path.join(folder,'serial.log'),text);
  const complete=log.complete===true && log.start_cursor!=null && log.end_cursor!=null && log.dropped_lines===0 && log.window_covered===true;
  return {...log,raw_base64:undefined,text,decoded,complete,truncated:log.truncated===true||log.lines.length>config.max_lines};
}
