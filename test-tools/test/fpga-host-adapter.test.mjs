import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { spawn } from 'node:child_process';
import { performance } from 'node:perf_hooks';
import { readManifest } from '../runner.mjs';
import { attachHostAdapter, createFpgaHostAdapter, FPGA_HOST_CAPABILITIES, FPGA_HOST_VERSION } from '../lib/host/fpga-host-adapter.mjs';
import { workspace, validateUi } from '../lib/hub/manifest.mjs';
import { ConfigStore } from '../lib/hub/config-store.mjs';
import { getProject, projectSnapshot } from '../lib/hub/project-config.mjs';
import { execute } from '../hub-workspace.mjs';

function tempDir(t) {
  const root = fs.mkdtempSync(path.join(os.tmpdir(), 'wiparse-fpga-host-'));
  t.after(() => fs.rmSync(root, { recursive: true, force: true }));
  return root;
}

function writePlugin(dir, extra = {}) {
  fs.mkdirSync(dir, { recursive: true });
  const manifest = { id: 'plain-plugin', name: 'Plain', type: 'custom', report_name: 'PlainReport', version: '1.0.0', ...extra };
  fs.writeFileSync(path.join(dir, 'plugin.json'), JSON.stringify(manifest));
  return manifest;
}

function eventsBus() {
  const listeners = new Set();
  let seq = 0;
  return {
    get seq() { return seq; },
    on(fn) { listeners.add(fn); },
    off(fn) { listeners.delete(fn); },
    emit(event) {
      seq += 1;
      const message = { ...event, seq };
      for (const fn of [...listeners]) fn(message);
    },
  };
}

test('hardware discovery excludes simulated instruments',async t=>{
  const root=tempDir(t);
  const invoke=async method=>method==='instrument.list'?{data:{devices:[{kind:'oscilloscope',resource:'DEMO::Oscilloscope',device_id:1},{kind:'usb_bridge',resource:'bridge://ft4222/serial=DEMO',device_id:2,identity:{firmware:'demo'}},{kind:'oscilloscope',resource:'USB::SCOPE',device_id:3,identity:{model:'MDO'}}]}}:method==='serial.ports'?{data:[]}:Promise.reject(Error('unexpected method'));
  const adapter=createFpgaHostAdapter({dataRoot:root,layout:{test_dir:path.join(root,'test')},invoke,events:eventsBus()});
  const devices=await adapter.devices();assert.deepEqual(devices.map(d=>d.resource),['USB::SCOPE']);
});

test('Dongle UART discovery requires an explicit WinUSB serial and exposes the real port',async t=>{
  const {resolveResources}=await import('../plugins/fpga-firmware-test/lib/resource-resolver.mjs');const root=tempDir(t);
  const invoke=async method=>method==='instrument.list'?{data:{devices:[]}}:method==='serial.ports'?{data:[{device:'winusb://wiparse/uart?serial=ABC123'},{device:'winusb://wiparse/uart'},{device:'COM4'}]}:Promise.reject(Error('unexpected method'));
  const adapter=createFpgaHostAdapter({dataRoot:root,layout:{test_dir:path.join(root,'test')},invoke,events:eventsBus()});
  const devices=await adapter.devices(),dongles=devices.filter(device=>device.kind==='wiparse_dongle_uart');assert.equal(dongles.length,1);
  assert.deepEqual(dongles[0],{kind:'wiparse_dongle_uart',port:'winusb://wiparse/uart?serial=ABC123',resource:'winusb://wiparse/uart?serial=ABC123',dongle_serial:'ABC123',uart_channel:0});
  const resolved=resolveResources({log:{kind:'wiparse_dongle_uart',selector:{dongle_serial:'ABC123',uart_channel:0,baud:115200},required:true}},devices);
  assert.equal(resolved.log.port,'winusb://wiparse/uart?serial=ABC123');
  assert.throws(()=>resolveResources({log:{kind:'wiparse_dongle_uart',selector:{dongle_serial:'ABC123',uart_channel:1},required:true}},devices),/RESOURCE.MISSING/);
});
test('resolved Dongle UART reuses an existing monitor and does not stop it on release',async t=>{
  const {resolveResources}=await import('../plugins/fpga-firmware-test/lib/resource-resolver.mjs');const root=tempDir(t),port='winusb://wiparse/uart?serial=ABC123',calls=[];
  const invoke=async(method,params)=>{calls.push(method);if(method==='instrument.list')return {data:{devices:[]}};if(method==='serial.ports')return {data:[{device:port}]};if(method==='serial.monitor.status')return {data:{sessions:[{port,tab_id:3}]}};if(method==='log.lines.get')return {data:{lines:[],next_row:0,dropped:0}};throw Error(`unexpected ${method}`);};
  const adapter=createFpgaHostAdapter({dataRoot:root,layout:{test_dir:path.join(root,'test')},invoke,events:eventsBus()});
  const resources=resolveResources({log:{kind:'wiparse_dongle_uart',selector:{dongle_serial:'ABC123',uart_channel:0,baud:115200},required:true}},await adapter.devices());
  const lease=await adapter.acquire({station_id:'one',run_id:'run',resources});assert.equal(lease.handoffs[0].port,port);assert.equal(lease.handoffs[0].started,false);
  await adapter.release(lease);assert(!calls.includes('serial.monitor.start'));assert(!calls.includes('serial.monitor.stop'));
});

test('I2C commands require the leased Dongle and confirmed matching job',async t=>{
  const root=tempDir(t),events=eventsBus();let calls=0;
  const invoke=async (method,params)=>{
    assert.equal(method,'instrument.command');calls++;
    events.emit({type:'instrument.job_done',data:{job_id:calls,ok:true,detail:{response:'OK READ 88 57'}}});
    return {data:{job_id:calls}};
  };
  const adapter=createFpgaHostAdapter({dataRoot:root,layout:{test_dir:path.join(root,'test')},invoke,events});
  const device={kind:'wiparse_dongle',serial:'ABC',device_id:42,purpose:'i2c'};
  const command={DongleI2cRead:{addr:'2B',len:2}};
  await assert.rejects(adapter.i2cCommand({resource:device,command}),/LEASE.REQUIRED/);
  const lease=await adapter.acquire({station_id:'S',run_id:'R',resources:{dongle:device}});
  await assert.rejects(adapter.i2cCommand({resource:{...device,device_id:43},command}),/LEASE.REQUIRED/);
  await assert.rejects(adapter.i2cCommand({resource:device,command:{ProbeReset:{}}}),/I2C.COMMAND_UNSUPPORTED/);
  const reply=await adapter.i2cCommand({resource:device,command});assert.equal(reply.data.detail.response,'OK READ 88 57');assert.equal(calls,1);
  await adapter.release(lease);await assert.rejects(adapter.assertI2cLease(device),/LEASE.REQUIRED/);
});

test('leased FT4222 routes MT5788 register reads through confirmed instrument jobs',async t=>{
  const {createFT4222Bus}=await import('../plugins/wireless-whitebox/ft4222-i2c-bus.mjs');
  const {createMT5788}=await import('../plugins/wireless-whitebox/mt5788.mjs');
  const root=tempDir(t),events=eventsBus();let job=0,reg=0;
  const invoke=async(method,params)=>{
    assert.equal(method,'instrument.command');const c=params.command.BridgeI2c;assert(c);job++;
    if(c.write_hex)reg=Number.parseInt(c.write_hex,16);
    const w=c.write_hex?c.write_hex.match(/../g).join(' ').toUpperCase():'—';
    const r=c.read_len?(reg===0?'88 57':'88 00'):'—';
    events.emit({type:'instrument.job_done',data:{job_id:job,ok:true,detail:{response:`I2C 0x2B @100000Hz write ${w} read ${r}`}}});
    return {data:{job_id:job}};
  };
  const adapter=createFpgaHostAdapter({dataRoot:root,layout:{test_dir:path.join(root,'test')},invoke,events});
  const resource={kind:'usb_bridge',resource:'bridge://ft4222/serial=ABC',device_id:12};
  const lease=await adapter.acquire({station_id:'S',run_id:'R',resources:{rx:resource}});
  const bus=createFT4222Bus({checkLease:()=>adapter.assertI2cLease(resource),executeConfirmed:command=>adapter.i2cCommand({resource,command})});
  const chip=createMT5788(bus);assert.equal((await chip.identify()).variant,'HW');assert.equal(job,4);
  await adapter.release(lease);await assert.rejects(chip.readRxTelemetry(),/LEASE.REQUIRED/);
});

test('load commands require the exact leased load and allow only confirmed CC operations',async t=>{
  const root=tempDir(t),events=eventsBus();let calls=0;
  const invoke=async(method,params)=>{
    assert.equal(method,'instrument.command');calls++;
    events.emit({type:'instrument.job_done',data:{job_id:calls,ok:true}});
    return {data:{job_id:calls}};
  };
  const host=createFpgaHostAdapter({dataRoot:root,layout:{test_dir:path.join(root,'test')},invoke,events});
  const load={kind:'electronic_load',resource:'USB::LOAD',device_id:5};
  await assert.rejects(host.loadCommand({resource:load,command:{LoadInput:false}}),/LEASE.REQUIRED/);
  const lease=await host.acquire({station_id:'S',run_id:'R',resources:{load}});
  await assert.rejects(host.loadCommand({resource:{...load,device_id:6},command:{LoadInput:false}}),/LEASE.REQUIRED/);
  await assert.rejects(host.loadCommand({resource:load,command:{LoadLevel:{mode:'CC',value:-1}}}),/LOAD.COMMAND_INVALID/);
  await assert.rejects(host.loadCommand({resource:load,command:{LoadMode:'CV'}}),/LOAD.COMMAND_INVALID/);
  for(const command of [{LoadInput:false},{LoadMode:'CC'},{LoadLevel:{mode:'CC',value:0.5}},{LoadInput:true}])await host.loadCommand({resource:load,command});
  assert.equal(calls,4);await host.release(lease);
});

test('unknown device state retains a recovery lock even if the owner is gone',async t=>{
  const root=tempDir(t),ctx={dataRoot:root,layout:{test_dir:path.join(root,'test')},invoke:async()=>{throw new Error('unexpected');}};
  const host=createFpgaHostAdapter(ctx),resource={kind:'usb_bridge',resource:'bridge://ft4222/serial=ABC',device_id:12};
  const lease=await host.acquire({station_id:'S',run_id:'R',resources:{rx:resource}});
  await host.markLeaseUncertain(lease);await assert.rejects(host.release(lease),/RECOVERY_REQUIRED/);
  const saved=JSON.parse(fs.readFileSync(lease.lock_path));saved.pid=99999999;fs.writeFileSync(lease.lock_path,JSON.stringify(saved));
  await assert.rejects(createFpgaHostAdapter(ctx).acquire({station_id:'S',run_id:'other',resources:{rx:resource}}),/RECOVERY_REQUIRED/);
  const other={...ctx,layout:{test_dir:path.join(root,'different-project')}};
  await assert.rejects(createFpgaHostAdapter(other).acquire({station_id:'different',run_id:'another',resources:{rx:resource}}),/RECOVERY_REQUIRED/);
});

test('power voltage selects the exact channel and V unit, not current or another channel',async t=>{
  const root=tempDir(t),events=eventsBus();let readings=[{channel:'CH1 Current',value:1.4,unit:'A'},{channel:'CH10 Voltage',value:50,unit:'V'},{channel:'CH1 Voltage',value:5.1,unit:'V'}];
  const invoke=async(method)=>{assert.equal(method,'instrument.measure');events.emit({type:'instrument.measurements',data:{device_id:2,readings}});return {ok:true};};
  const adapter=createFpgaHostAdapter({dataRoot:root,invoke,events}),resource={kind:'dc_source',device_id:2};
  assert.equal(await adapter.voltage(resource,1),5.1);
  readings=[{channel:'CH1 Current',value:1.4,unit:'A'},{channel:'CH10 Voltage',value:50,unit:'V'}];
  await assert.rejects(adapter.voltage(resource,1),/MEASURE.EMPTY/);
  readings=[{channel:'CH1 Voltage',value:5.1,unit:'V'},{channel:'CH1 Voltage',value:5.2,unit:'V'}];
  await assert.rejects(adapter.voltage(resource,1),/MEASURE.EMPTY/);
});

test('a new active RX message permits readiness without reusing old serial rows',async t=>{
  const root=tempDir(t);let page=0;
  const invoke=async(method,params)=>{
    if(method==='serial.monitor.status')return {data:{sessions:[]}};
    if(method==='serial.monitor.start')return {data:{tab_id:0,port:'COM7'}};
    if(method==='serial.monitor.stop')return {data:{}};
    if(method==='log.lines.get'){
      page++;if(page===1)return {data:{lines:['ASK 31 old'],next_row:1,dropped:0}};
      if(page===2){assert.equal(params.from_row,1);return {data:{lines:['ASK 310','ASK 31A'],next_row:3,dropped:0}};}
      assert.equal(params.from_row,3);return {data:{lines:['ASK 31 new'],next_row:4,dropped:0}};
    }
    throw new Error(method);
  };
  const host=createFpgaHostAdapter({dataRoot:root,layout:{test_dir:path.join(root,'test')},invoke});
  const log={kind:'serial_port',port:'COM7'},lease=await host.acquire({station_id:'S',run_id:'R',resources:{log}});
  const ready=await host.waitSerialSequence({resource:log,markers:['Start CHG','ASK 71'],active_marker:'ASK 31',timeout_ms:1000});
  assert.equal(ready.events.length,1);assert.equal(ready.events[0].line,'ASK 31 new');await host.release(lease);
});

test('plugins without host_adapter do not receive fpgaAdapter', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wiparse-no-adapter-'));
  try {
    writePlugin(dir);
    const plugin = readManifest(dir);
    assert.equal(plugin.host_adapter, undefined);
    assert.equal(Object.hasOwn(plugin, 'host_adapter'), false);
    const ctx = { plugin, url: 'http://127.0.0.1:9' };
    attachHostAdapter(ctx);
    assert.equal(ctx.fpgaAdapter, undefined);
    const other = { plugin: { ...plugin, host_adapter: 'other/1' }, url: 'http://127.0.0.1:9' };
    attachHostAdapter(other);
    assert.equal(other.fpgaAdapter, undefined);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('host_adapter wiparse.fpga-host/1 selects createFpgaHostAdapter', () => {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'wiparse-adapter-'));
  try {
    writePlugin(dir, { host_adapter: FPGA_HOST_VERSION });
    const plugin = readManifest(dir);
    assert.equal(plugin.host_adapter, FPGA_HOST_VERSION);
    const ctx = { plugin, url: 'http://127.0.0.1:9', dataRoot: dir };
    attachHostAdapter(ctx);
    const direct = createFpgaHostAdapter(ctx);
    assert.equal(ctx.fpgaAdapter.version, direct.version);
    assert.equal(ctx.fpgaAdapter.version, FPGA_HOST_VERSION);
    assert.deepEqual(ctx.fpgaAdapter.capabilities, FPGA_HOST_CAPABILITIES);
    for (const method of ['devices', 'preflight', 'storage', 'acquire', 'release', 'sourceOutput', 'voltage', 'probeReset', 'collectLog', 'capture', 'waveformSource', 'cleanup', 'cancelAndConfirm', 'measure']) {
      assert.equal(typeof ctx.fpgaAdapter[method], 'function', method);
    }
    assert.equal(ctx.fpgaAdapter.ai, undefined);
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test('station lease is exclusive, stolen from a dead pid, and released only by the owner', async (t) => {
  const root = tempDir(t);
  const testDir = path.join(root, 'test');
  const make = () => createFpgaHostAdapter({ dataRoot: root, layout: { test_dir: testDir }, invoke: async () => { throw new Error('invoke should not run'); } });
  const first = make();
  const lease = await first.acquire({ station_id: 'Station A', run_id: 'run-1', resources: [{ kind: 'dc_source', resource: 'TCPIP::PSU' }] });
  assert.equal(lease.pid, process.pid);
  assert.equal(JSON.parse(fs.readFileSync(lease.lock_path, 'utf8')).pid, process.pid);
  const second = make();
  await assert.rejects(second.acquire({ station_id: 'Station A', run_id: 'run-2', resources: [] }), /STATION\.LOCKED/);
  await first.release(lease);
  assert.equal(fs.existsSync(lease.lock_path), false);
  const again = await second.acquire({ station_id: 'Station A', run_id: 'run-3', resources: [] });
  await second.release(again);

  const child = spawn(process.execPath, ['-e', 'setTimeout(()=>{}, 30000)'], { stdio: 'ignore', windowsHide: true });
  await new Promise((resolve, reject) => { child.once('spawn', resolve); child.once('error', reject); });
  const dead = child.pid;
  child.kill();
  await new Promise(resolve => child.once('exit', resolve));
  const lock = path.join(testDir, '.leases', 'station-Dead.json');
  fs.mkdirSync(path.dirname(lock), { recursive: true });
  fs.writeFileSync(lock, JSON.stringify({ pid: dead, station_id: 'Dead' }));
  const stolen = await make().acquire({ station_id: 'Dead', run_id: 'recovered', resources: [] });
  assert.equal(JSON.parse(fs.readFileSync(stolen.lock_path, 'utf8')).pid, process.pid);
  fs.writeFileSync(stolen.lock_path, JSON.stringify({ pid: dead, station_id: 'Dead' }));
  await assert.rejects(make().release(stolen), /LEASE\.PID_MISMATCH/);
  assert.equal(fs.existsSync(stolen.lock_path), true);
});

test('preflight reports a missing resource instead of HOST.ADAPTER_REQUIRED', async (t) => {
  const root = tempDir(t);
  const invoke = async method => {
    if (method === 'instrument.list') return { ok: true, data: { devices: [{ device_id: 3, kind: 'oscilloscope', resource: 'TCPIP::SCOPE', identity: { model: 'MDO', serial: 'SN1' } }] } };
    if (method === 'serial.ports') return { ok: true, data: [{ device: 'COM4', description: 'USB Serial' }] };
    throw new Error(`unexpected ${method}`);
  };
  const adapter = createFpgaHostAdapter({ dataRoot: root, invoke, events: eventsBus() });
  const devices = await adapter.devices();
  assert.equal(devices.length, 2);
  assert.equal(devices[0].model, 'MDO');
  assert.equal(devices[0].serial, 'SN1');
  assert.equal(devices[1].kind, 'serial_port');
  assert.equal(devices[1].port, 'COM4');
  await adapter.preflight({ resources: { scope: devices[0] }, identity: { station_id: 'S' }, station: { station_id: 'S' } });
  await assert.rejects(adapter.preflight({ resources: { missing: { kind: 'dc_source', serial: 'NOPE' } }, identity: { station_id: 'S' } }), error => error.code === 'RESOURCE.MISSING' && !String(error.message).includes('HOST.ADAPTER_REQUIRED'));
});

test('collectLog base64 matches the cursor span and stays incomplete when stopped', async (t) => {
  const root = tempDir(t);
  const events = eventsBus();
  const lines = [];
  let owned = false;
  let starts = 0;
  const invoke = async (method, params) => {
    if (method === 'serial.monitor.status') return { ok: true, data: { monitoring: owned, port: owned ? 'COM7' : null, baud: 115200, status: owned ? 'open COM7 @ 115200' : 'stopped' } };
    if (method === 'serial.monitor.start') { starts += 1; owned = true; return { ok: true, data: { monitoring: true, port: params.port, baud: params.baud, tab_id: 0, already_open: false } }; }
    if (method === 'serial.monitor.stop') { owned = false; return { ok: true, data: { stopped: true } }; }
    if (method === 'log.lines.get') {
      assert.equal(params.port, 'COM7');
      const from = Number(params.from_row) || 0;
      const page = lines.slice(from);
      return { ok: true, data: { lines: page, next_row: lines.length, dropped: 0, tab_id: 0, port: 'COM7' } };
    }
    throw new Error(`unexpected ${method}`);
  };
  const adapter = createFpgaHostAdapter({ dataRoot: root, layout: { test_dir: path.join(root, 'test') }, invoke, events });
  const resource = { kind: 'serial_port', port: 'COM7' };
  const lease = await adapter.acquire({ station_id: 'LOG', run_id: '1', resources: { log: resource } });
  assert.equal(starts, 1);
  assert.equal(lease.cursors[Object.keys(lease.cursors)[0]], 0);
  await adapter.release(lease);
  owned = true;
  const handed = await adapter.acquire({ station_id: 'LOG', run_id: '2', resources: { log: resource } });
  assert.equal(starts, 1, 'an existing handoff must not start a second owner');
  lines.push('BOOT');
  let stop = false;
  const start = performance.now();
  const partial = await adapter.collectLog({ resource, start_mono_ms: start, end_mono_ms: start + 500, stopped: () => stop, dir: root });
  stop = true;
  const stoppedLog = await adapter.collectLog({ resource, start_mono_ms: performance.now(), end_mono_ms: performance.now() + 500, stopped: () => true, dir: root });
  const raw = Buffer.from(partial.raw_base64, 'base64');
  assert.equal(raw.toString('utf8'), 'BOOT\n');
  assert.equal(partial.end_cursor - partial.start_cursor, raw.length);
  assert.equal(partial.complete, false);
  assert.equal(partial.provenance, 'reconstructed_text');
  assert.equal(partial.timestamp_source, 'host_poll');
  assert(partial.quality_issues.includes('LOG.RAW_BYTES_UNAVAILABLE'));
  const {preserveLog} = await import('../plugins/fpga-firmware-test/lib/log-capture.mjs');
  const saved = preserveLog(partial, path.join(root, 'evidence'), {max_lines:1000});
  assert.equal(saved.complete, false);
  assert(fs.existsSync(path.join(root, 'evidence', 'logs', 'serial.reconstructed.txt')));
  assert.equal(fs.existsSync(path.join(root, 'evidence', 'logs', 'serial.raw')), false);
  assert.equal(stoppedLog.complete, false);
  assert.equal(stoppedLog.end_cursor, stoppedLog.start_cursor + Buffer.from(stoppedLog.raw_base64, 'base64').length);
  await adapter.release(handed);
});

test('cancelAndConfirm stays unsettled until the instrument job finishes', async (t) => {
  const root = tempDir(t);
  const events = eventsBus();
  let finish = () => {};
  const hold = new Promise(resolve => { finish = resolve; });
  const invoke = async (method, params) => {
    if (method !== 'instrument.command') throw new Error(method);
    hold.then(() => events.emit({ type: 'instrument.job_done', data: { job_id: 4, ok: true, kind: 'command', detail: {} } }));
    assert.deepEqual(params.command, { SourceOutput: { channel: 1, enabled: true } });
    return { ok: true, data: { accepted: true, job_id: 4 } };
  };
  const adapter = createFpgaHostAdapter({ dataRoot: root, invoke, events });
  const running = adapter.sourceOutput({ kind: 'dc_source', resource: 'TCPIP::PSU', device_id: 2 }, 1, true, { operation_id: 'op-1', timeout_s: 5 });
  await new Promise(resolve => setTimeout(resolve, 20));
  let quiescent = false;
  const confirmation = adapter.cancelAndConfirm({ operation_id: 'op-1', method: 'sourceOutput', reason: 'IO.TIMEOUT' }).then(result => { quiescent = true; return result; });
  await new Promise(resolve => setTimeout(resolve, 40));
  assert.equal(quiescent, false);
  finish();
  assert.deepEqual(await confirmation, { quiescent: true });
  await running;
});

test('handoff starts an extra COM while another session is already live', async (t) => {
  const root = tempDir(t);
  const calls = [];
  const invoke = async (method, params) => {
    calls.push({ method, params });
    if (method === 'serial.monitor.status') return { ok: true, data: { monitoring: true, port: 'COM3', baud: 115200, tab_id: 0, sessions: [{ port: 'COM3', tab_id: 0 }] } };
    if (method === 'serial.monitor.start') return { ok: true, data: { monitoring: true, port: params.port, baud: params.baud, tab_id: 2, already_open: false } };
    if (method === 'log.lines.get') {
      assert.equal(params.port, 'COM7');
      return { ok: true, data: { lines: [], next_row: 0, dropped: 0, tab_id: 2, port: 'COM7' } };
    }
    throw new Error(`unexpected ${method}`);
  };
  const adapter = createFpgaHostAdapter({ dataRoot: root, layout: { test_dir: path.join(root, 'test') }, invoke, events: eventsBus() });
  const lease = await adapter.acquire({ station_id: 'EXTRA', run_id: '1', resources: { log: { kind: 'serial_port', port: 'COM7' } } });
  assert.equal(lease.handoffs[0].port, 'COM7');
  assert.equal(lease.handoffs[0].tab_id, 2);
  assert.equal(lease.handoffs[0].started, true);
  assert.equal(calls.some(call => call.method === 'serial.monitor.start' && call.params.port === 'COM7'), true);
});

test('waveformSource forwards measured metrics and does not invent missing ones', async (t) => {
  const root = tempDir(t);
  const events = eventsBus();
  const waves = path.join(root, 'waves');
  fs.mkdirSync(waves, { recursive: true });
  const dest = path.join(waves, 'case.isf');
  fs.writeFileSync(dest, 'isf');
  const invoke = async (method) => {
    if (method !== 'instrument.waveform_source') throw new Error(method);
    events.emit({
      type: 'instrument.job_done',
      data: {
        job_id: 9,
        ok: true,
        kind: 'waveform_source',
        detail: {
          path: dest,
          metrics: { CH1: { frequency_hz: { value: 1000, unit: 'Hz', quality: 'good' }, duty_cycle: { value: 0.5, unit: 'ratio', quality: 'good' } } },
        },
      },
    });
    return { ok: true, data: { accepted: true, job_id: 9 } };
  };
  const adapter = createFpgaHostAdapter({ dataRoot: root, invoke, events });
  const measured = await adapter.waveformSource({ resource: { kind: 'oscilloscope', device_id: 1 }, config: { id: 'case' }, dir: root });
  assert.equal(measured.metrics.CH1.frequency_hz.value, 1000);
  assert.equal(measured.metrics.CH1.duty_cycle.unit, 'ratio');
  assert.equal(measured.metrics.CH1.rise_time_s, undefined);

  const emptyEvents = eventsBus();
  const emptyInvoke = async (method) => {
    if (method !== 'instrument.waveform_source') throw new Error(method);
    emptyEvents.emit({ type: 'instrument.job_done', data: { job_id: 10, ok: true, kind: 'waveform_source', detail: { path: dest } } });
    return { ok: true, data: { accepted: true, job_id: 10 } };
  };
  const silent = createFpgaHostAdapter({ dataRoot: root, invoke: emptyInvoke, events: emptyEvents });
  const omitted = await silent.waveformSource({ resource: { kind: 'oscilloscope', device_id: 1 }, config: { id: 'case' }, dir: root });
  assert.equal(omitted.metrics, undefined);
});

test('storage reports a real free-space number for data_root', async (t) => {
  const root = tempDir(t);
  const adapter = createFpgaHostAdapter({ dataRoot: root, invoke: async () => { throw new Error('unused'); } });
  const result = await adapter.storage({ minimum_free_bytes: 1 });
  assert.equal(result.ok, true);
  assert.equal(Number.isFinite(result.free_bytes), true);
  assert.ok(result.free_bytes >= 1);
  await assert.rejects(adapter.storage({ minimum_free_bytes: Number.MAX_SAFE_INTEGER }), /STORAGE\./);
});

function seedPlugin(dir) {
  fs.mkdirSync(dir, { recursive: true });
  fs.writeFileSync(path.join(dir, 'index.mjs'), 'export function configure(){return {ok:true,errors:[],warnings:[]};}\nexport function configureProject(){return {ok:true,errors:[],warnings:[]};}\n');
  fs.writeFileSync(path.join(dir, 'config.schema.json'), JSON.stringify({ type: 'object', properties: { id: { type: 'string' }, name: { type: 'string' } }, required: ['id'], additionalProperties: true }));
  fs.writeFileSync(path.join(dir, 'project.schema.json'), JSON.stringify({ type: 'object', properties: { name: { type: 'string', minLength: 1 } }, required: ['name'], additionalProperties: false }));
  return {
    id: 'seed-demo', name: 'Seed', type: 'custom', version: '1.0.0', report_name: 'SeedDemo', entry: 'index.mjs',
    capabilities: ['configure', 'preflight', 'run'],
    params: [{ name: 'config_dir', type: 'path', default: 'station/cases' }],
    ui: {
      mode: 'test_matrix', max_items: 20, matrix_status_file: '{test_dir}/matrix.json',
      config_editor: { schema: 'config.schema.json', repository: '{config_dir}' },
      project_config: { protocol: 'wiparse.project-config/1', schema: 'project.schema.json', seed: '{config_dir}/../project.json' },
    },
  };
}

test('project seed is an unsaved draft in the config root and preflight still requires a save', t => {
  const root = tempDir(t);
  const pluginDir = path.join(root, 'plugin');
  const plugin = seedPlugin(pluginDir);
  fs.writeFileSync(path.join(pluginDir, 'plugin.json'), JSON.stringify(plugin));
  const dataRoot = path.join(root, 'data');
  fs.mkdirSync(path.join(dataRoot, 'station', 'cases'), { recursive: true });
  fs.writeFileSync(path.join(dataRoot, 'station', 'project.json'), JSON.stringify({ name: 'Seeded project' }));
  const context = { pluginDir, dataRoot, project: 'lot', testId: 'SeedDemo', params: { config_dir: 'station/cases' } };
  const store = new ConfigStore(workspace(plugin, context));
  const draft = getProject(store);
  assert.equal(draft.saved, false);
  assert.equal(draft.seed_draft, true);
  assert.equal(draft.value.name, 'Seeded project');
  assert.equal(fs.existsSync(store.s.projectFile), false);
  assert.throws(() => projectSnapshot(store), /PROJECT\.REQUIRED/);
  const opened = execute({ context, method: 'workspace.open', params: {}, internal: true });
  assert.equal(opened.project_config.saved, false);
  assert.equal(opened.project_config.seed_draft, true);
  assert.equal(opened.project_config.value.name, 'Seeded project');
  assert.equal(fs.existsSync(store.s.projectFile), false);
  plugin.ui.project_config.seed = '{config_dir}/../../project.json';
  assert.throws(() => workspace(plugin, context), /PLUGIN\.SEED_PATH/);
  plugin.ui.project_config.seed = 'C:/outside/project.json';
  assert.throws(() => validateUi(plugin), /PLUGIN\.SEED_PATH/);
  plugin.ui.project_config.seed = '{config_dir}/../project.json';
  plugin.ui.project_config.schema = '../project.schema.json';
  assert.throws(() => validateUi(plugin), /PLUGIN\.UI_SCHEMA_INVALID/);
});
