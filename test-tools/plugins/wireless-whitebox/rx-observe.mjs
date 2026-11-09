import {performance} from 'node:perf_hooks';
import {randomUUID} from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import {createHttpClient} from '../../lib/wiparse-sdk.mjs';
import {createFT4222Bus} from './ft4222-i2c-bus.mjs';
import {createMT5788} from './mt5788.mjs';
import {withConstantCurrentLoad} from './load-sequence.mjs';
import {serialPortsFromReply} from './station-readiness.mjs';

export async function resolveBridge(ctx) {
  const station=ctx.projectConfig?.value?.station||{};
  if(!String(station.station_id||'').trim())throw new Error('STATION.ID_REQUIRED');
  const target=String(station.rx_i2c_interface||'').trim();
  if(!target)throw new Error('RX.FT4222_REQUIRED');
  const port=String(station.log_source||'').trim(),baud=station.log_baud??115200;
  if(!port||!Number.isInteger(baud)||baud<1||baud>10000000)throw new Error('RX.TX_LOG_REQUIRED');
  const markers=[station.rx_start_marker??'Start CHG',station.rx_identification_marker??'ASK 71'];
  const active_marker=station.rx_running_marker??'ASK 31';
  const timeout_s=station.rx_ready_timeout_s??30;
  if([...markers,active_marker].some(m=>typeof m!=='string'||!m.trim()||m.length>128)||!Number.isInteger(timeout_s)||timeout_s<1||timeout_s>300)throw new Error('RX.READY_CONFIG_INVALID');
  const invoke=ctx.invoke??createHttpClient({url:ctx.url,log:ctx.log}).invoke;
  const r=await invoke('instrument.list',{},5000), list=r?.data?.devices??r?.devices;
  if(!Array.isArray(list))throw new Error('INSTRUMENT.LIST_INVALID');
  const matches=list.filter(d=>d.resource===target && ['usbbridge','bridge'].includes(String(d.kind||'').toLowerCase().replace(/[^a-z]/g,'')));
  if(matches.length!==1 || !Number.isSafeInteger(matches[0].device_id))throw new Error('RX.FT4222_NOT_CONNECTED');
  const demo=d=>String(d.resource||'').startsWith('DEMO::')||String(d.identity?.firmware||'').toLowerCase()==='demo';
  if(demo(matches[0]))throw new Error('RX.DEMO_DEVICE_NOT_ALLOWED');
  let scope=null;
  if(station.scope_source_enabled===true && station.scope_capture_enabled!==true)throw new Error('SCOPE.SCREENSHOT_REQUIRED');
  if(station.scope_capture_enabled===true) {
    if(!String(station.scope_resource||'').trim())throw new Error('SCOPE.RESOURCE_REQUIRED');
    const scopes=list.filter(d=>d.resource===station.scope_resource && String(d.kind||'').toLowerCase().replace(/[^a-z]/g,'')==='oscilloscope');
    if(scopes.length!==1||!Number.isSafeInteger(scopes[0].device_id))throw new Error('SCOPE.NOT_CONNECTED');
    if(demo(scopes[0]))throw new Error('SCOPE.DEMO_DEVICE_NOT_ALLOWED');
    scope={kind:'oscilloscope',resource:scopes[0].resource,serial:scopes[0].identity?.serial||'',device_id:scopes[0].device_id};
  }
  let load=null;
  const measurement=ctx.testMatrix?.values?.find(v=>v.id==='wcv_07')?.physical_vout_limit;
  if(measurement) {
    if(!String(station.load_resource||'').trim())throw new Error('LOAD.RESOURCE_REQUIRED');
    const loads=list.filter(d=>d.resource===station.load_resource && String(d.kind||'').toLowerCase().replace(/[^a-z]/g,'')==='electronicload');
    if(loads.length!==1||!Number.isSafeInteger(loads[0].device_id))throw new Error('LOAD.NOT_CONNECTED');
    if(demo(loads[0]))throw new Error('LOAD.DEMO_DEVICE_NOT_ALLOWED');
    load={kind:'electronic_load',resource:loads[0].resource,serial:loads[0].identity?.serial||'',device_id:loads[0].device_id};
  }
  const portsReply=await invoke('serial.ports',{},5000);let ports;
  try {ports=serialPortsFromReply(portsReply);} catch {throw new Error('RX.TX_LOG_PORT_UNAVAILABLE');}
  if(!ports.some(p=>String(p.device??p.port??'').toUpperCase()===port.toUpperCase()))throw new Error('RX.TX_LOG_PORT_UNAVAILABLE');
  const d=matches[0];
  return {resource:{kind:'usb_bridge',resource:target,serial:d.identity?.serial||d.serial||'',device_id:d.device_id},scope,load,measurement,scope_source_enabled:station.scope_source_enabled===true,log:{kind:'serial_port',port,baud},markers,active_marker,timeout_ms:timeout_s*1000,station_id:station.station_id,clock_hz:station.rx_i2c_clock_hz??100000};
}

// With no control argument only register pointers are written. With control,
// the configured MT5788 VOUT target is changed; power/load outputs are not.
export async function observeRx(ctx,{stopped=()=>false,control=null,load=null}={}) {
  const bound=await resolveBridge(ctx), host=ctx.fpgaAdapter;
  if(!host || typeof host.i2cCommand!=='function' || typeof host.assertI2cLease!=='function'||typeof host.waitSerialSequence!=='function')throw new Error('RX.HOST_ADAPTER_REQUIRED');
  const lease=await host.acquire({station_id:bound.station_id,run_id:ctx.testMatrix.run_id,resources:{rx:bound.resource,log:bound.log,...(bound.scope?{scope:bound.scope}:{}),...(bound.load?{load:bound.load}:{})}});
  let unknown=false,caught=null,controlApplied=false;
  const partial={};
  try {
    const readiness=await host.waitSerialSequence({resource:bound.log,markers:bound.markers,active_marker:bound.active_marker,timeout_ms:bound.timeout_ms,stopped});
    partial.readiness=readiness;
    const executeGuarded=async (fn,{allowStopped=false}={})=>{
      const operation_id=randomUUID();
      if(stopped()&&!allowStopped)throw new Error('RUN.ABORTED');
      try {return await fn({operation_id,deadline_mono_ms:performance.now()+15000});}
      catch(e) {
        if(e.quiescent!==true) {
          const confirmation=await Promise.race([host.cancelAndConfirm({operation_id}).catch(()=>null),new Promise(resolve=>setTimeout(()=>resolve(null),1000))]);
          if(confirmation?.quiescent!==true) {
            unknown=true;
            if(typeof host.markLeaseUncertain!=='function')throw new Error('RX.RECOVERY_MARKER_UNAVAILABLE');
            await host.markLeaseUncertain(lease,'I2C_STATE_UNKNOWN');
            throw new Error('RX.I2C_STATE_UNKNOWN_MANUAL_RECOVERY');
          }
        }
        throw e;
      }
    };
    const bus=createFT4222Bus({clock_hz:bound.clock_hz,checkLease:()=>{if(stopped())throw new Error('RUN.ABORTED');return host.assertI2cLease(bound.resource);},executeConfirmed:command=>executeGuarded(control=>host.i2cCommand({resource:bound.resource,command,...control}))});
    const chip=createMT5788(bus),identity=await chip.identify();let telemetry=await chip.readRxTelemetry(),controlResult=null;
    if(control) {
      try {
        controlResult=await chip.setVoutMv(control.target_mv,{minimum_mv:control.minimum_mv,maximum_mv:control.maximum_mv});
        controlApplied=true;
        telemetry=await chip.readRxTelemetry();
      } catch(error) {
        if(!unknown) {
          unknown=true;
          if(typeof host.markLeaseUncertain==='function')await host.markLeaseUncertain(lease,'RX_OUTPUT_STATE_UNKNOWN');
        }
        throw error;
      }
    }
    partial.identity=identity;partial.telemetry=telemetry;partial.control=controlResult;
    const captureMeasurements=async()=>{
    if(stopped())throw new Error('RUN.ABORTED');
    let physical_measurement=null;
    if(bound.load) {
      const until=performance.now()+bound.measurement.settle_ms;
      while(performance.now()<until) {if(stopped())throw new Error('RUN.ABORTED');await new Promise(resolve=>setTimeout(resolve,Math.min(50,Math.max(1,until-performance.now()))));}
      await host.assertInstrumentLease(bound.load);
      const measured=await executeGuarded(c=>host.measure({resource:bound.load,unit:'V',...c}));
      if(measured?.unit!=='V'||!Number.isFinite(measured.value))throw new Error('LOAD.VOLTAGE_INVALID');
      physical_measurement={metric:'rx_vout',value:measured.value,unit:'V',source:bound.load.resource,observed_at:new Date().toISOString(),quality:'instrument_reported'};
      partial.physical_measurement=physical_measurement;
    }
    const captures={};
    if(bound.scope) {
      const dir=path.join(ctx.layout.artifacts_dir,'reports','items','wcv_07');
      const artifact=value=>{
        if(!value?.path||!value?.sha256)throw new Error('SCOPE.ARTIFACT_MISSING');
        const root=fs.realpathSync(dir),file=fs.realpathSync(value.path),relative=path.relative(root,file);
        if(relative.startsWith('..')||path.isAbsolute(relative))throw new Error('SCOPE.ARTIFACT_PATH');
        return {path:relative.split(path.sep).join('/'),sha256:value.sha256};
      };
      const screenshot=await executeGuarded(control=>host.capture({resource:bound.scope,config:{id:'rx-observe'},dir,...control}));
      captures.screenshot=artifact(screenshot.screenshot);
      if(bound.scope_source_enabled) {
        const source=await executeGuarded(control=>host.waveformSource({resource:bound.scope,config:{id:'rx-observe'},dir,...control}));
        captures.waveform=artifact(source.artifact);
      }
    }
    return {physical_measurement,captures};
    };
    let collected,loadResult=null;
    if(load) {
      if(!bound.load||typeof host.loadCommand!=='function')throw new Error('LOAD.HOST_ADAPTER_REQUIRED');
      const sequence=await withConstantCurrentLoad({current_a:load.current_a,maximum_a:load.maximum_a,stopped,isUncertain:()=>unknown,
        command:command=>executeGuarded(c=>host.loadCommand({resource:bound.load,command,...c}),{allowStopped:command.LoadInput===false}),
        markUncertain:async reason=>{if(!unknown){unknown=true;await host.markLeaseUncertain(lease,reason);}}
      },captureMeasurements);
      collected=sequence.value;loadResult=sequence.load;partial.load=loadResult;
    } else collected=await captureMeasurements();
    return {schema_version:'wiparse.mt5788-observation/1',observed_at:new Date().toISOString(),identity,telemetry,readiness,captures:collected.captures,control:controlResult,load:loadResult,physical_measurement:collected.physical_measurement,source_resource:bound.resource.resource,product_verdict:'INCONCLUSIVE',note:'日志协议事件表明 RX 已响应；电压和波形依次采集，不保证与协议事件同步'};
  } catch(error) {
    if(controlApplied && !partial.physical_measurement && !unknown) {
      unknown=true;
      try {await host.markLeaseUncertain(lease,'RX_OUTPUT_UNVERIFIED');}catch(e){error.recovery_error=e.message;}
    }
    error.partial_observation=partial;
    caught=error;
    throw error;
  } finally {
    try {if(!unknown)await host.release(lease);}
    catch(error) {
      if(caught)caught.cleanup_error=error.message;
      else {error.partial_observation=partial;throw error;}
    } finally {host.closeEvents?.();}
  }
}
