import fs from 'node:fs';
import path from 'node:path';
import {randomUUID} from 'node:crypto';
import {readDocument} from './config.mjs';
import {aggregate,stateRule,communicationRule} from './rules.mjs';
import {writeReport} from './report.mjs';
const mono=()=>performance.now();
export async function execute(bundle,{root,dir,client,signal,stopRequested=()=>false,project='default',test_id='WirelessCharge',run_id=randomUUID(),attempt_id=randomUUID(),fixtureDocument}={}) {
  const run={schema_version:bundle.profile.schema_version,run_id,attempt_id,project,test_id,dut:{serial:bundle.args.dut_serial||null,firmware:bundle.args.dut_firmware||null},plugin_version:'0.1.1',configuration_hashes:bundle.hashes,mode:bundle.args.mode,execution_status:'completed',product_verdict:'INCONCLUSIVE',validation_verdict:'INCONCLUSIVE',industrial_approved:false,cases:[],errors:[],first_error_id:null};
  const error=(code,c,check,refs=[],serviceError=null)=>{
    const e={error_id:randomUUID(),code,source:'wireless-charge-validation',severity:code.includes('FAIL')?'product_fault':'data_or_station',run_id,step_id:c?.case_id||null,check_id:check?.check_id||null,time:new Date().toISOString(),host_monotonic_ns:process.hrtime.bigint().toString(),first_error_id:run.first_error_id,observed:check?.product_verdict||null,expected:'PASS',automatic_action:'no_stimulus',recovery_id:'review-evidence-new-attempt',evidence_refs:refs,candidate_cause:code,suggested_action:'保留首个异常及原始证据；核对限值/状态/采集质量。未知设备状态不可重发刺激。'};
    e.service_error_record=serviceError;
    if(!run.first_error_id) run.first_error_id=e.error_id;
    e.first_error_id=run.first_error_id;run.errors.push(e);return e;
  };
  const guard=()=>{if(signal?.aborted||stopRequested()) throw new Error('RUN.CANCELLED');};
  let fixture;
  try {
    guard();
    if(bundle.args.mode==='live') throw new Error('INTEGRATION.LIVE_BACKEND_NOT_VALIDATED');
    if(bundle.args.mode==='replay') {
      const document=fixtureDocument||readDocument(root,`tests/fixtures/${bundle.args.fixture}.json`);
      fixture=document.value;run.fixture_hash=document.sha256;
      if(fixture.schema_version!==bundle.profile.schema_version||fixture.simulation_only!==true) throw new Error('REPLAY.UNVERIFIED_PROVENANCE');
      fs.mkdirSync(dir,{recursive:true});fs.writeFileSync(path.join(dir,'replay-input.json'),JSON.stringify(fixture,null,2)+'\n');
    }
    for(const scenario of bundle.cases) {
      guard();
      const c={case_id:scenario.id,sys:scenario.sys,pl:scenario.pl,dg:scenario.dg,sw:scenario.sw,execution_status:'completed',product_verdict:'INCONCLUSIVE',checks:[],reason_code:null};run.cases.push(c);
      const end=mono()+scenario.deadline_ms;
      const bounded=async(method,params)=>{
        guard();const remaining=Math.floor(end-mono());if(remaining<=0)throw new Error('STEP.DEADLINE');
        const r=await client.request('/v1/invoke',{method,params},remaining);
        if(mono()>end)throw new Error('STEP.DEADLINE');return r;
      };
      try {
        for(const spec of scenario.checks) {
          guard();let result={product_verdict:'INCONCLUSIVE',reason_code:'DRY_RUN.NO_MEASUREMENT',evidence_refs:[]};
          const o=fixture?.cases?.[scenario.id]?.[spec.id];
          if(bundle.args.mode==='replay'&&o) {
            if(spec.kind==='scalar') {
              const item=bundle.limits.items[spec.limits_ref];
              const m=o.measurement;
              const valid=m?.schema_version==='wiparse.automation/1'&&m?.metric_id===spec.metric_id&&m.unit===item.unit&&m.evidence_refs?.length&&m.window?.length===2&&m.window.every(Number.isFinite)&&m.window[1]>m.window[0]&&o.state===spec.state&&m.channel===spec.channel&&(!spec.window_s||m.window.every((v,i)=>v===spec.window_s[i]));
              result=valid?await bounded('check.evaluate',{measurement:m,limits:{limits_ref:spec.limits_ref,limits_hash:bundle.hashes.limits,approved:bundle.limits.approved,unit:item.unit,minimum:item.minimum,maximum:item.maximum,require_uncertainty:true}}):{product_verdict:'INCONCLUSIVE',reason_code:'EVIDENCE.STATE_CHANNEL_WINDOW_OR_METRIC_MISSING',evidence_refs:m?.evidence_refs||[]};
              if(!['PASS','FAIL','INCONCLUSIVE','SKIP'].includes(result.product_verdict)) throw new Error('CHECK.INVALID_SERVICE_RESPONSE');
            } else if(spec.kind==='state') result=stateRule(spec,o,bundle.states);
            else result=communicationRule(spec,o);
          } else if(bundle.args.mode==='replay') result.reason_code='REPLAY.MISSING_REQUIRED_EVIDENCE';
          const check={...result,check_id:spec.id,case_id:scenario.id,sys:scenario.sys,pl:scenario.pl,sw:scenario.sw,dg:scenario.dg,limits_hash:bundle.hashes.limits,measurement:o?.measurement||null,state_window:spec.state||null,scope:bundle.args.mode==='replay'?'simulation':'dry-run'};
          c.checks.push(check);
          if(check.product_verdict!=='PASS')error(check.product_verdict==='FAIL'?`PRODUCT.FAIL:${check.reason_code}`:check.reason_code,c,check,check.evidence_refs);
          // Preserve a completed FAIL even if stop arrived during this read-only check.
          guard();
        }
        c.product_verdict=aggregate(c.checks);c.reason_code=c.checks.find(x=>x.product_verdict!=='PASS')?.reason_code||'CHECKS.COMPLETE';
      } catch(e) {
        c.execution_status=e.message==='RUN.CANCELLED'?'cancelled':e.message.includes('UNKNOWN')?'interrupted':'error';
        c.product_verdict=aggregate(c.checks,c.execution_status);c.reason_code=e.message;throw e;
      }
    }
  } catch(e) {
    run.execution_status=e.message==='RUN.CANCELLED'?'cancelled':e.message.includes('UNKNOWN')?'interrupted':'error';error(e.message,run.cases.at(-1),null,[],e.error_record||null);
  }
  // Every uncompleted selected case remains explicitly present in coverage.
  for(const scenario of bundle.cases) if(!run.cases.some(c=>c.case_id===scenario.id)) run.cases.push({case_id:scenario.id,sys:scenario.sys,pl:scenario.pl,sw:scenario.sw,dg:scenario.dg,execution_status:run.execution_status,product_verdict:'INCONCLUSIVE',checks:[],reason_code:'RUN.NOT_EXECUTED'});
  run.validation_verdict=aggregate(run.cases,run.execution_status);
  // Simulation/dry-run never certifies a DUT, even when its validation database is all green.
  run.product_verdict='INCONCLUSIVE';
  return {...run,...writeReport(dir,bundle,run),ok:run.execution_status==='completed'};
}
