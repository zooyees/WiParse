import path from 'node:path';
import fs from 'node:fs';
import {atomicWrite} from '../../lib/hub/common.mjs';
export async function configure(ctx){return {ok:true,lifecycle:'configure',errors:[],warnings:[],normalized_value:null};}
export async function preflight(ctx){return {ok:true,test_matrix:{manifest_sha256:ctx.testMatrix.manifest_sha256,revision:ctx.testMatrix.manifest_sha256,item_count:ctx.testMatrix.selected_item_ids.length}};}
export async function stop(ctx){atomicWrite(path.join(ctx.layout.test_dir,'hub.stop'),'stop');return {ok:true};}
export default async function run(ctx){
  const binding=ctx.testMatrix, selected=new Set(binding.selected_item_ids);
  const stopFile=path.join(ctx.layout.test_dir,'hub.stop');if(fs.existsSync(stopFile))fs.unlinkSync(stopFile);
  const items=binding.values.map(v=>({id:v.id,name:v.name,tags:v.tags,order:v.order||0,selected:selected.has(v.id),status:selected.has(v.id)?'pending':'skipped',phase:'waiting',attempt:1,progress:0,elapsed_s:0,message:'',result_ref:null}));
  let revision=0;
  const publish=()=>atomicWrite(ctx.matrixFile,JSON.stringify({schema_version:'wiparse.test-matrix/1',plugin_id:ctx.plugin.id,revision:++revision,run_id:binding.run_id,preflight_revision:binding.preflight_revision,updated_at:new Date().toISOString(),active_item_id:items.find(i=>i.status==='running')?.id||null,summary:{total:items.length,selected:items.filter(i=>i.selected).length,...Object.fromEntries(['pending','running','pass','fail','error','skipped','aborted','inconclusive'].map(s=>[s,items.filter(i=>i.status===s).length]))},items}));
  publish();for(const row of items.filter(i=>i.selected)){
    row.status='running';row.phase='inspect';publish();
    const until=Date.now()+Math.max(10,binding.values.find(v=>v.id===row.id).duration_s*1000);
    while(Date.now()<until&&!fs.existsSync(stopFile))await new Promise(resolve=>setTimeout(resolve,10));
    if(fs.existsSync(stopFile)){row.status='aborted';row.phase='stopped';publish();break;}
    row.status='pass';row.progress=1;row.phase='completed';publish();
  }
  const report=path.join(ctx.layout.artifacts_dir,'reports','summary.md');atomicWrite(report,'# Workspace simulation\n\n'+items.filter(i=>i.selected).map(i=>`- ${i.name}: PASS`).join('\n'));
  return {ok:true,summary_md:report,test_matrix:{manifest_sha256:binding.manifest_sha256},execution_status:'completed',product_verdict:'NOT_EVALUATED'};
}
