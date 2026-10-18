import fs from 'node:fs';
import path from 'node:path';
import {jail,readJson,atomicWrite,fail} from './common.mjs';
export function controlState(store){
  if(!store.s.controlFile)return {supported:false};
  const lock=store.runLocks();
  if(!lock)return {supported:true,state:'idle'};
  let value={run_id:lock.run_id,state:'running'};
  if(fs.existsSync(store.s.controlFile)){const saved=readJson(jail(store.s.controlFile,[store.s.testDir]));if(saved.run_id===lock.run_id)value=saved;}
  return {supported:true,...value};
}
export function requestControl(store,params){
  if(!store.s.controlFile)fail('RUN.CONTROL_UNSUPPORTED');
  const lock=store.runLocks();
  if(!lock||lock.run_id!==params.run_id)fail('RUN.CONTROL_STALE');
  if(!['pause','resume'].includes(params.action))fail('RUN.CONTROL_INVALID');
  const state=controlState(store);
  if(params.action==='pause'&&!['running','resume_requested'].includes(state.state))fail('RUN.CONTROL_INVALID_STATE');
  if(params.action==='resume'&&!['pause_requested','paused'].includes(state.state))fail('RUN.CONTROL_INVALID_STATE');
  const value={run_id:lock.run_id,action:params.action,state:params.action==='pause'?'pause_requested':'resume_requested',updated_at:new Date().toISOString()};
  atomicWrite(jail(store.s.controlFile,[store.s.testDir]),JSON.stringify(value));return {supported:true,...value};
}
