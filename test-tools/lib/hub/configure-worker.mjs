import {pathToFileURL} from 'node:url';
import path from 'node:path';
import {jail} from './common.mjs';
let input=''; for await (const chunk of process.stdin) { input+=chunk; if (Buffer.byteLength(input)>41943040) throw new Error('Configure input too large'); }
const request=JSON.parse(input);
const output=process.stdout.write.bind(process.stdout);
// Plugin logging must not corrupt the bounded protocol response.
console.log=console.info=console.warn=console.error=()=>{};
process.stdout.write=()=>true;
try {
  const entry=jail(path.resolve(request.pluginDir,request.entry),[request.pluginDir]);
  const mod=await import(pathToFileURL(entry).href);
  if(request.migration){if(typeof mod.planProjectMigration!=='function')throw new Error('Migration unsupported');const plan=await mod.planProjectMigration({cases:request.values,projectConfig:request.projectConfig});output(JSON.stringify(plan));process.exit(0);}
  if(request.projectOnly){const result=typeof mod.configureProject==='function'?await mod.configureProject({lifecycle:'configure',preflightOnly:true,projectConfig:request.projectConfig,overlay:{project_config:request.projectConfig},args:{},log:()=>{},api:()=>{throw new Error('No device access during configure');}}):{ok:true,errors:[],warnings:[]};output(JSON.stringify(result));process.exit(0);}
  if (typeof mod.configure!=='function') throw new Error('configure export required');
  const errors=[],warnings=[],normalized_values=[];
  for (const value of request.values) {
    const result=await mod.configure({lifecycle:'configure',preflightOnly:true,projectConfig:request.projectConfig,overlay:{config:structuredClone(value),project_config:request.projectConfig},args:{},log:()=>{},api:()=>{throw new Error('No device access during configure');}});
    if (!result || typeof result!=='object' || result.lifecycle!=='configure' || !Array.isArray(result.errors || []) || !Array.isArray(result.warnings || [])) throw new Error('Invalid configure result');
    if (result.ok===false || result.errors?.length) errors.push(...(result.errors?.length?result.errors:[{path:'/',message:'Plugin rejected configuration'}]));
    warnings.push(...(result.warnings || []));
    if(request.values.length===1)normalized_values.push(result.normalized_value??null);
    if (errors.length>100 || warnings.length>100) break;
  }
  output(JSON.stringify({ok:!errors.length,errors:errors.slice(0,100),warnings:warnings.slice(0,100),normalized_values}));
} catch { output(JSON.stringify({ok:false,errors:[{path:'/',message:'Configure lifecycle failed'}]})); process.exitCode=1; }
