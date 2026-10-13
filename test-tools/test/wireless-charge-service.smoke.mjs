// Real HTTP service + Testing Hub runner, no hardware connections or power stimuli.
import assert from 'node:assert/strict';
import {mkdtemp,readFile,mkdir} from 'node:fs/promises';
import {spawn} from 'node:child_process';
import {once} from 'node:events';
import {createServer} from 'node:net';
import path from 'node:path';
import {AutomationClient} from '../lib/automation-client.mjs';
const root=path.resolve(process.argv[3]||'evidence/wireless-plugin-2026-10-11');
await mkdir(root,{recursive:true});
const work=await mkdtemp(path.join(root,'http-run-'));
const listener=createServer();await new Promise(r=>listener.listen(0,'127.0.0.1',r));const port=listener.address().port;await new Promise(r=>listener.close(r));
const service=spawn(path.resolve(process.argv[2]||'target/release/wiparse-service.exe'),['--data-root',path.join(work,'automation-data'),'--port',String(port)],{windowsHide:true,stdio:['ignore','pipe','pipe']});
let diagnostics='',spawnError;service.stderr.on('data',b=>diagnostics+=b);service.stdout.on('data',()=>{});service.on('error',e=>spawnError=e);
const client=new AutomationClient(path.join(work,'automation-data'));
async function runner(lifecycle,fixture,mode='replay',expect=0) {
  const p=spawn(process.execPath,['test-tools/runner.mjs','--plugin','wireless-charge-validation','--lifecycle',lifecycle,'--data-root',work,'--project','smoke','--stamp',fixture,'--','--suite','regression','--mode',mode,'--fixture',fixture,'--service_root',path.join(work,'automation-data')],{windowsHide:true,stdio:['ignore','pipe','pipe']});
  let out='';p.stdout.on('data',b=>out+=b);p.stderr.on('data',b=>out+=b);const [code]=await once(p,'exit');assert.equal(code,expect,out);return out;
}
const start=performance.now();
try {
  while(true){if(spawnError)throw spawnError;if(service.exitCode!==null)throw new Error(diagnostics);try{await client.request('/v1/health');break;}catch(e){if(performance.now()-start>10000)throw e;await new Promise(r=>setTimeout(r,50));}}
  await runner('preflight','gold');
  const counts=[];
  for(const [fixture,expected] of [['gold','PASS'],['bad','FAIL'],['missing','INCONCLUSIVE'],['quality','INCONCLUSIVE']]) {
    await runner('run',fixture);
    const dir=path.join(work,'projects/smoke/tests/WirelessCharge/runs',fixture);
    const report=JSON.parse(await readFile(path.join(dir,'artifacts/reports/checks.json'),'utf8'));
    assert.equal(report.validation_verdict,expected);assert.equal(report.product_verdict,'INCONCLUSIVE');assert.equal(report.execution_status,'completed');
    const metadata=JSON.parse(await readFile(path.join(dir,'run.json'),'utf8'));
    assert.equal(metadata.product_verdict,'INCONCLUSIVE');assert.equal(metadata.validation_verdict,expected);
    counts.push({fixture,cases:report.cases.length,checks:report.cases.flatMap(c=>c.checks).length,validation_verdict:expected});
  }
  await runner('preflight','gold','live',1);
  await runner('stop','gold');
  await client.request('/v1/stop',{});if(service.exitCode===null)await once(service,'exit');assert.equal(service.exitCode,0,diagnostics);
  console.log(JSON.stringify({ok:true,industrial_approved:false,service_exe:path.resolve(process.argv[2]||'target/release/wiparse-service.exe'),evidence_root:work,fixtures:counts},null,2));
} finally {
  if(!spawnError&&service.exitCode===null){service.kill();await once(service,'exit').catch(()=>{});}
  // Evidence intentionally retained; token/lease data must stay local.
}
