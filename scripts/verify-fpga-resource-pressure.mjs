/** Isolated log preservation pressure probe. Does not claim hardware/aging timing. */
import fs from 'node:fs';
import path from 'node:path';
import assert from 'node:assert/strict';
import {performance} from 'node:perf_hooks';
import {preserveLog} from '../test-tools/plugins/fpga-firmware-test/lib/log-capture.mjs';

const root=path.resolve(process.argv[2]??`tmp/fpga-log-pressure-${Date.now()}`);
if(fs.existsSync(root))throw Error('Choose a new output directory');
fs.mkdirSync(root,{recursive:true});
const count=200000;
global.gc?.();const baseline=process.memoryUsage();
let lines=Array.from({length:count},(_,i)=>({mono_ms:i,utc:'2026-10-18T00:00:00.000Z',text:`SIMULATION UART ${i} STATUS=OK`}));
let raw=Buffer.from(lines.map(v=>v.text).join('\n')+'\n');
const inputBytes=raw.length,started=performance.now();
let result=preserveLog({raw_base64:raw.toString('base64'),lines,start_cursor:0,end_cursor:inputBytes,dropped_lines:0,window_covered:true,complete:true},root,{max_lines:count});
const elapsed=performance.now()-started,loaded=process.memoryUsage();
assert.equal(result.complete,true);assert.equal(result.truncated,false);
assert.equal(fs.statSync(path.join(root,'logs/serial.raw')).size,inputBytes);
const jsonl=fs.readFileSync(path.join(root,'logs/serial.jsonl'),'utf8');
assert.equal(jsonl.trimEnd().split('\n').length,count);
assert.equal(JSON.parse(jsonl.trimEnd().split('\n').at(-1)).mono_ms,count-1);
result=null;lines=null;raw=null;global.gc?.();
const summary={ok:true,simulation_only:true,node:process.version,rows:count,raw_bytes:inputBytes,elapsed_ms:elapsed,
 lines_per_second:count/(elapsed/1000),baseline,loaded,after_gc:process.memoryUsage(),max_rss_bytes:process.resourceUsage().maxRSS*1024,
 scope:'Synchronous preservation inside the isolated worker; excludes real serial ingress, device cancellation, T0 precision and longest-duration aging'};
fs.writeFileSync(path.join(root,'verification.json'),JSON.stringify(summary,null,2));console.log(JSON.stringify(summary,null,2));
