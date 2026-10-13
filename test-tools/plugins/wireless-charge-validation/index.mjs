import fs from 'node:fs';
import path from 'node:path';
import {randomUUID} from 'node:crypto';
import {AutomationClient} from '../../lib/automation-client.mjs';
import {resolveProjectLayout,sanitizeTestId,resolveReportName} from '../../lib/plugin-contract.mjs';
import {loadBundle,preflightBundle,inside} from './lib/config.mjs';
import {execute} from './lib/orchestrator.mjs';
function setup(ctx) {
  const root=ctx.pluginDir||ctx.plugin.dir;
  const bundle=loadBundle(root,ctx.args);
  const layout=ctx.layout||resolveProjectLayout({dataRoot:ctx.dataRoot,project:ctx.project,testId:sanitizeTestId(resolveReportName(ctx.plugin,ctx.args)),pluginId:ctx.plugin.id,stamp:ctx.stamp,lifecycle:ctx.lifecycle||'run'});
  const client=ctx.automationClient||new AutomationClient(path.resolve(ctx.args?.service_root||process.env.WIPARSE_SERVICE_ROOT||path.join(ctx.dataRoot,'automation-data')));
  return {root,bundle,layout,client};
}
export async function preflight(ctx) {
  const {bundle,client}=setup({...ctx,lifecycle:'preflight'});
  let caps={implemented:[],industrial_approved:false};
  if(bundle.args.mode!=='dry-run') {
    try {caps=await client.invoke('system.capabilities');} catch(e) {return {ok:false,preflight:{blocking:[e.message],product_verdict:'INCONCLUSIVE'},error:e.message};}
    if(bundle.args.mode==='replay'&&!caps.implemented?.includes('check.evaluate')) return {ok:false,error:'CAPABILITY.MISSING:check.evaluate'};
  }
  const result=preflightBundle(bundle,caps);
  return {ok:result.ok,preflight:result,error:result.blocking.join(';')||undefined};
}
export async function stop(ctx) {
  // No config/run allocation and no stale stop file for a later attempt.
  const layout=ctx.layout;
  if(!layout?.test_dir||!fs.existsSync(path.join(layout.test_dir,'wireless.active.json'))) return {ok:true,step:'stop',preflight:{active:false}};
  const active=JSON.parse(fs.readFileSync(path.join(layout.test_dir,'wireless.active.json'),'utf8'));
  fs.writeFileSync(path.join(layout.test_dir,'wireless.stop.json'),JSON.stringify({attempt:active.attempt}));
  return {ok:true,step:'stop'};
}
export default async function run(ctx) {
  if(ctx.lifecycle==='stop')return stop(ctx);
  if(ctx.lifecycle==='preflight')return preflight(ctx);
  const {root,bundle,layout,client}=setup(ctx);
  if(!layout.run_dir||!layout.test_dir)throw new Error('RUN.LAYOUT_MISSING');
  const dir=inside(path.resolve(layout.run_dir,'artifacts/reports'),path.resolve(layout.run_dir));
  // Reusing a second-resolution stamp must not replace the first failure/evidence.
  if(fs.existsSync(path.join(dir,'manifest.json')))throw new Error('RUN.EXISTING_ATTEMPT');
  fs.mkdirSync(layout.test_dir,{recursive:true});
  const lock=path.join(layout.test_dir,'wireless.active.json'),stopFile=path.join(layout.test_dir,'wireless.stop.json');
  const attempt=randomUUID();
  let fd;try {fd=fs.openSync(lock,'wx');}catch(e){if(e.code==='EEXIST')throw new Error('RUN.BUSY_OR_STALE_LOCK');throw e;}
  try {
    fs.writeFileSync(fd,JSON.stringify({attempt,run_dir:layout.run_dir}));fs.closeSync(fd);fd=undefined;
    return await execute(bundle,{root,dir,client,attempt_id:attempt,project:layout.project,test_id:layout.test_id,signal:ctx.signal,stopRequested:()=>fs.existsSync(stopFile)&&JSON.parse(fs.readFileSync(stopFile,'utf8')).attempt===attempt});
  } finally {
    if(fd!==undefined)fs.closeSync(fd);
    if(fs.existsSync(stopFile))fs.unlinkSync(stopFile);
    fs.unlinkSync(lock);
  }
}
