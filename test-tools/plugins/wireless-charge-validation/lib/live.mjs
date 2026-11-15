import {randomUUID} from 'node:crypto';
import {hash} from './config.mjs';

// One fenced case. The station owns its approved sequence, hardware watchdog and output feedback.
export async function captureCase(bundle,scenario,run,request,guard) {
  // Until the external service exposes an enforceable per-run RX readiness
  // gate, do not issue a lease or any hardware request through this entry.
  guard();
  throw new Error('INTEGRATION.TX_RX_FT4222_GATE_NOT_IMPLEMENTED');
  const base={station_id:bundle.station.station_id,case_id:scenario.id,run_id:run.run_id,attempt_id:run.attempt_id,
    dut_serial:run.dut.serial,firmware:run.dut.firmware,configuration_hashes:bundle.hashes};
  const deadline=performance.now()+scenario.deadline_ms;
  let lease,primary;
  const call=async(method,p,cleanup=false)=>{
    if(!cleanup)guard();
    const remaining=cleanup?3000:Math.floor(deadline-performance.now());
    if(remaining<1)throw Error('STEP.DEADLINE');
    const record={case_id:scenario.id,method,command_id:p.command_id,requested_at:new Date().toISOString()};
    run.station_activity??=[];run.station_activity.push(record);
    try{
      const reply=await request(method,{...base,...lease,...p},remaining);
      record.replied_at=new Date().toISOString();
      for(const key of ['capture_id','state','armed','protection_healthy','feedback_verified','outputs_safe','released']){
        if(['boolean','string'].includes(typeof reply[key]))record[key]=reply[key];
      }
      return reply;
    }catch(e){record.error=e.message;throw e;}

  };
  const bound=(r)=>r && ['station_id','case_id','run_id','attempt_id','dut_serial','firmware'].every(k=>r[k]===base[k]);
  try {
    const resource=bundle.station.live.resource;
    const grant=await call('device.lease.acquire',{resource,ttl_s:Math.ceil(scenario.deadline_ms/1000)+10});
    if(grant.resource!==resource || !grant.lease_token || !grant.session_epoch)throw Error('LEASE.INVALID_GRANT');
    lease={resource,lease_token:grant.lease_token,session_epoch:grant.session_epoch};
    const arm=await call('capture.arm',{command_id:randomUUID(),capture_plan:scenario.capture_plan});
    if(!bound(arm)||!arm.capture_id)throw Error('CAPTURE.ARM_UNVERIFIED');
    const capture={capture_id:arm.capture_id};
    const dongle=await call('dongle.capture.start',{...capture,command_id:randomUUID()});
    const status=await call('capture.status',capture);
    if(!bound(dongle)||dongle.capture_id!==arm.capture_id || dongle.ready!==true || !bound(status) ||
        status.capture_id!==arm.capture_id || !['armed','protection_healthy','watchdog_active','safe_before_stimulus'].every(k=>status[k]===true))
      throw Error('SAFETY.ARM_OR_PROTECTION_UNVERIFIED');
    const command_id=randomUUID();
    const change=await call('stimulus.apply',{...capture,command_id});
    if(!bound(change)||change.command_id!==command_id||change.capture_id!==arm.capture_id||change.feedback_verified!==true)throw Error('STIMULUS.OUTCOME_UNKNOWN');
    let out;
    do{
      out=await call('capture.status',capture);
      if(!bound(out)||out.capture_id!==arm.capture_id||out.protection_healthy!==true)throw Error('CAPTURE.IDENTITY_OR_PROTECTION_LOST');
      if(out.state==='complete')break;
      if(out.state!=='acquiring')throw Error('CAPTURE.UNKNOWN_STATE');
      await new Promise(resolve=>setTimeout(resolve,50));
    }while(true);
    if(out.quality_validated!==true||out.simulation_only!==false||!out.observations||!out.evidence)throw Error('CAPTURE.INVALID_EVIDENCE');
    for(const check of scenario.checks){
      const o=out.observations[check.id];
      const refs=o?.measurement?.evidence_refs??o?.evidence_refs??o?.events?.flatMap(e=>e.evidence_refs??[]);
      if(!Array.isArray(refs)||!refs.length||!refs.every(key=>{
        const e=Object.hasOwn(out.evidence,key)&&out.evidence[key];
        return e?.capture_id===arm.capture_id && typeof e.source==='string' && e.source.trim() && Number.isSafeInteger(e.position) &&
          ['line_1based','sample_0based','byte_0based'].includes(e.coordinate) && e.position>=(e.coordinate==='line_1based'?1:0) &&
          typeof e.raw==='string' && e.raw.length>0 && hash(e.raw)===e.sha256;
      }))throw Error('CAPTURE.UNGROUNDED_OBSERVATION');
    }
    run.live_captures??={};run.live_captures[scenario.id]={capture_id:out.capture_id,evidence:out.evidence,observations:out.observations};
    return out;
  }catch(error){primary=error;throw error;}
  finally{
    if(lease){
      let stopped;
      try{
        stopped=await call('station.safe_stop',{command_id:randomUUID()},true);
        if(!bound(stopped)||stopped.outputs_safe!==true||stopped.feedback_verified!==true)throw Error('SAFETY.STOP_OUTCOME_UNKNOWN');
        await call('device.lease.release',{},true);
      }catch(error){
        run.cleanup_errors??=[];run.cleanup_errors.push({case_id:scenario.id,error:error.message});
        if(!primary)throw error;
      }
    }
  }
}
