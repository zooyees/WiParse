/**
 * Host implementation of wiparse.fpga-host/1.
 * Instrument work is confirmed on the GUI event stream. A caller timeout does
 * not drop that wait: cancelAndConfirm observes the same completion.
 */
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';
import { spawnSync } from 'node:child_process';
import { performance } from 'node:perf_hooks';
import { createHttpClient, localApiHeaders } from '../wiparse-sdk.mjs';
import {atomicWrite} from '../hub/common.mjs';

export const FPGA_HOST_VERSION = 'wiparse.fpga-host/1';
export const FPGA_HOST_CAPABILITIES = ['resource_lease', 'serial_handoff_raw_cursor', 'timed_capture', 'safe_cleanup', 'storage_check'];

const SERIAL_KINDS = new Set(['serial_port', 'wiparse_dongle', 'wiparse_dongle_uart']);

function coded(code, detail) {
  const error = new Error(detail ? `${code}: ${detail}` : code);
  error.code = code;
  return error;
}

function pidAlive(pid) {
  if (!Number.isInteger(pid) || pid <= 0) return false;
  try {
    process.kill(pid, 0);
    return true;
  } catch (error) {
    return error.code === 'EPERM';
  }
}

function budgetMs(control, fallback = 120_000) {
  let ms = fallback;
  if (control && Number.isFinite(control.deadline_mono_ms)) ms = Math.min(ms, control.deadline_mono_ms - performance.now());
  const configured = Number(control?.config?.timeout_s ?? control?.timeout_s);
  if (Number.isFinite(configured) && configured > 0) ms = Math.min(ms, configured * 1000);
  if (!Number.isFinite(ms) || ms < 1) throw coded('IO.TIMEOUT', 'no time remains in the configured budget');
  return Math.floor(ms);
}

function controlFrom(args) {
  const last = args[args.length - 1];
  if (last && typeof last === 'object' && (last.operation_id || last.deadline_mono_ms || last.signal)) return last;
  return {};
}

function payload(result) {
  return result?.data ?? result ?? {};
}

function asList(data, key) {
  if (Array.isArray(data)) return data;
  if (Array.isArray(data?.[key])) return data[key];
  return [];
}

function resourceKey(resource) {
  if (!resource || typeof resource !== 'object') return '';
  return ['kind', 'serial', 'resource', 'port', 'dongle_serial'].map(key => resource[key] ?? '').join(':');
}

function flattenResources(resources) {
  if (!resources) return [];
  if (Array.isArray(resources)) return resources.flatMap(flattenResources);
  if (typeof resources !== 'object') return [];
  if (resources.kind || resources.port || resources.serial || resources.resource || resources.dongle_serial) return [resources];
  return Object.values(resources).flatMap(flattenResources);
}

function sameDevice(live, wanted) {
  if (!wanted || typeof wanted !== 'object') return false;
  if (wanted.kind && live.kind && wanted.kind !== live.kind) return false;
  const keys = ['serial', 'resource', 'port', 'dongle_serial'].filter(key => wanted[key] != null && wanted[key] !== '');
  return keys.length > 0 && keys.every(key => live[key] === wanted[key]);
}

function deviceIdOf(resource) {
  const id = resource?.device_id;
  if (typeof id === 'number' && Number.isInteger(id) && id >= 0) return id;
  if (typeof id === 'string' && /^\d+$/.test(id)) return Number(id);
  throw coded('RESOURCE.UNRESOLVED', 'instrument device_id is missing');
}

function channelMatches(reading, channel) {
  if (channel == null || channel === '') return true;
  const want = String(channel).toUpperCase();
  const got = String(reading?.channel ?? '').toUpperCase();
  const expected=/^(?:CH)?(\d+)$/.exec(want);
  const actual=/^CH(\d+)(?:\s|$)/.exec(got);
  return got === want || (expected && actual && expected[1]===actual[1]);
}

function safeStem(value, ext) {
  const stem = String(value || 'capture').replace(/[^A-Za-z0-9._-]+/g, '_').slice(0, 80) || 'capture';
  return stem.toLowerCase().endsWith(ext) ? stem : `${stem}${ext}`;
}

function sha256(bytes) {
  return crypto.createHash('sha256').update(bytes).digest('hex');
}

function leaseRoot(ctx) {
  if (ctx.layout?.test_dir) return path.join(ctx.layout.test_dir, '.leases');
  if (ctx.dataRoot) return path.join(ctx.dataRoot, '.leases');
  throw coded('LEASE.PATH', 'data root is required');
}

function lockName(stationId) {
  const station = String(stationId ?? '').trim();
  const safe = (station || 'station').replace(/[^A-Za-z0-9._-]+/g, '_').slice(0, 120);
  return `station-${safe || 'station'}.json`;
}

function answeredFailure(error) {
  return /HTTP 4\d\d| failed:/.test(String(error?.message || error));
}

function freeBytes(dir) {
  if (typeof fs.statfsSync === 'function') {
    try {
      const stat = fs.statfsSync(dir);
      const avail = BigInt(stat.bavail);
      const size = BigInt(stat.bsize);
      const free = avail * size;
      if (free < 0n) throw coded('STORAGE.UNKNOWN', 'negative free space');
      return free;
    } catch (error) {
      if (error.code === 'STORAGE.UNKNOWN') throw error;
    }
  }
  if (process.platform === 'win32') {
    const quoted = `'${String(dir).replaceAll("'", "''")}'`;
    const script = `$p=(Resolve-Path -LiteralPath ${quoted}).Path; $q=Split-Path -Qualifier $p; if (-not $q) { exit 3 }; $d=New-Object System.IO.DriveInfo $q; if (-not $d.IsReady) { exit 4 }; [Console]::Out.Write($d.AvailableFreeSpace)`;
    const result = spawnSync('powershell.exe', ['-NoProfile', '-NonInteractive', '-Command', script], { encoding: 'utf8', windowsHide: true, timeout: 20_000 });
    const text = String(result.stdout || '').trim();
    if (result.status !== 0 || !/^\d+$/.test(text)) throw coded('STORAGE.UNKNOWN', (result.stderr || result.stdout || 'free space unavailable').trim());
    return BigInt(text);
  }
  throw coded('STORAGE.UNKNOWN', 'free space is unavailable on this host');
}

function createHttpEvents(url) {
  const base = String(url || '').replace(/\/$/, '');
  const listeners = new Set();
  let seq = 0;
  let started = false;
  let stopped = false;
  let controller = null;
  const publish = event => {
    for (const fn of [...listeners]) fn(event);
  };
  async function loop() {
    const decoder = new TextDecoder();
    let failures = 0;
    while (!stopped) {
      try {
        if (!base) throw coded('IO.UNCONFIRMED', 'WIPARSE_URL is not set');
        controller = new AbortController();
        const response = await fetch(`${base}/v1/events?since_seq=${seq}`,{signal:controller.signal,headers:localApiHeaders(base)});
        if (!response.ok || !response.body) throw coded('IO.UNCONFIRMED', `event stream HTTP ${response.status}`);
        failures = 0;
        const reader = response.body.getReader();
        let buffer = '';
        while (!stopped) {
          const chunk = await reader.read();
          if (chunk.done) break;
          buffer += decoder.decode(chunk.value, { stream: true });
          let newline = buffer.indexOf('\n');
          while (newline >= 0) {
            const line = buffer.slice(0, newline).trim();
            buffer = buffer.slice(newline + 1);
            newline = buffer.indexOf('\n');
            if (!line.startsWith('{')) continue;
            let event;
            try { event = JSON.parse(line); } catch { continue; }
            if (Number.isFinite(event.seq)) seq = event.seq;
            publish(event);
          }
        }
      } catch (error) {
        if(stopped) break;
        failures += 1;
        if (failures >= 3) {
          publish({ type: 'events.error', error });
          failures = 0;
        }
        await new Promise(resolve => setTimeout(resolve, 200));
      }
    }
  }
  return {
    get seq() { return seq; },
    on(fn) { listeners.add(fn); if (!started) { started = true; loop(); } },
    off(fn) { listeners.delete(fn); },
    stop() { stopped = true; controller?.abort(); },
  };
}

function captureEvents(events) {
  const since = Number(events.seq) || 0;
  const buffered = [];
  let predicate = null;
  let settled = false;
  let resolve;
  let reject;
  const promise = new Promise((ok, fail) => { resolve = ok; reject = fail; });
  const onEvent = event => {
    if (settled) return;
    if (event?.type === 'events.error') {
      settled = true;
      events.off(onEvent);
      reject(event.error || coded('IO.UNCONFIRMED', 'event stream failed'));
      return;
    }
    if (Number(event?.seq) > since) buffered.push(event);
    if (!predicate || !(Number(event?.seq) > since) || !predicate(event)) return;
    settled = true;
    events.off(onEvent);
    resolve(event);
  };
  events.on(onEvent);
  return {
    arm(next) {
      predicate = next;
      const found = buffered.find(event => Number(event?.seq) > since && next(event));
      if (!found || settled) return;
      settled = true;
      events.off(onEvent);
      resolve(found);
    },
    promise,
    cancel() { if (!settled) events.off(onEvent); },
  };
}

export function createFpgaHostAdapter(ctx) {
  const http = ctx.invoke ? null : createHttpClient({ url: ctx.url, log: ctx.log });
  const invoke = ctx.invoke ?? ((method, params, ms) => http.invoke(method, params, ms));
  const events = ctx.events ?? createHttpEvents(ctx.url);
  const inflight = new Map();
  const completed = new Map();
  let activeLease = null;

  function finish(operation, confirmed) {
    if (operation.done) return;
    operation.done = true;
    operation.confirmed = confirmed;
    completed.set(operation.id, confirmed);
    operation.resolve({ confirmed });
    inflight.delete(operation.id);
  }

  function track(control, method) {
    const id = control?.operation_id || crypto.randomUUID();
    let resolve;
    const operation = { id, method, done: false, confirmed: false, resolve, settled: null };
    operation.settled = new Promise(ok => { operation.resolve = ok; });
    inflight.set(id, operation);
    return operation;
  }

  async function call(method, params, control) {
    const ms = budgetMs(control);
    return invoke(method, params, ms);
  }

  function mapDevice(device) {
    const identity = device.identity && typeof device.identity === 'object' ? device.identity : {};
    if(String(device.resource||'').startsWith('DEMO::')||String(identity.firmware||'').toLowerCase()==='demo')return null;
    const row = { kind: String(device.kind || '') };
    if (device.device_id != null) row.device_id = device.device_id;
    const serial = device.serial || identity.serial || '';
    const resource = device.resource || '';
    const model = device.model || identity.model || '';
    if (serial) row.serial = String(serial);
    if (resource) row.resource = String(resource);
    if (model) row.model = String(model);
    if (row.kind === 'wiparse_dongle' && serial) row.dongle_serial = String(serial);
    if (device.dongle_serial) row.dongle_serial = String(device.dongle_serial);
    if (!row.serial && !row.resource && !row.port && !row.dongle_serial) return null;
    return row;
  }

  function mapPort(port) {
    const name = port.device || port.port;
    if (!name) return null;
    const row = { kind: 'serial_port', port: String(name), resource: String(name) };
    if (port.description) row.model = String(port.description);
    if (port.instance) row.dongle_serial = String(port.instance);
    return row;
  }

  function mapDongleUart(port) {
    // The WinUSB enumeration exposes one UART interface and a physical USB
    // serial in this exact URI. Do not infer an identity from a generic COM,
    // Wi-Fi name, or a USB entry without a stable serial.
    const name=String(port.device||port.port||'');
    const match=/^winusb:\/\/wiparse\/uart\?serial=([A-Za-z0-9._-]{1,128})$/i.exec(name);
    if(!match)return null;
    return {kind:'wiparse_dongle_uart',port:name,resource:name,dongle_serial:match[1],uart_channel:0};
  }

  async function devices() {
    const instruments = payload(await call('instrument.list', {}, null));
    const ports = payload(await call('serial.ports', {}, null));
    const portRows=asList(ports,'ports');
    return [...asList(instruments, 'devices').map(mapDevice),...portRows.map(mapPort),...portRows.map(mapDongleUart)].filter(Boolean);
  }

  function lockPath(stationId) {
    return path.join(leaseRoot(ctx), lockName(stationId));
  }

  function claim(file, body) {
    fs.mkdirSync(path.dirname(file), { recursive: true });
    for (let attempt = 0; attempt < 3; attempt += 1) {
      try {
        const fd = fs.openSync(file, 'wx');
        try { fs.writeFileSync(fd, JSON.stringify(body)); } finally { fs.closeSync(fd); }
        return;
      } catch (error) {
        if (error.code !== 'EEXIST') throw error;
        let existing = null;
        try { existing = JSON.parse(fs.readFileSync(file, 'utf8')); } catch { existing = null; }
        if(existing?.manual_recovery_required===true) throw coded('STATION.RECOVERY_REQUIRED','设备状态未知，需要人工确认');
        if (Number.isInteger(existing?.pid) && existing.pid === process.pid) throw coded('STATION.LOCKED', 'this process already holds the station');
        if (Number.isInteger(existing?.pid) && pidAlive(existing.pid)) throw coded('STATION.LOCKED', `held by pid ${existing.pid}`);
        try { fs.unlinkSync(file); } catch (unlinkError) { if (unlinkError.code !== 'ENOENT') throw unlinkError; }
      }
    }
    throw coded('STATION.LOCKED', 'could not claim the station lock');
  }

  function samePort(left, right) {
    return Boolean(left) && Boolean(right) && String(left).toUpperCase() === String(right).toUpperCase();
  }

  function sessionFor(status, port) {
    const sessions = Array.isArray(status?.sessions) ? status.sessions : [];
    const hit = sessions.find(session => samePort(session?.port, port));
    if (hit) return { port: hit.port, tab_id: Number.isInteger(hit.tab_id) ? hit.tab_id : 0 };
    if (samePort(status?.port, port)) return { port: status.port, tab_id: Number.isInteger(status.tab_id) ? status.tab_id : 0 };
    return null;
  }

  function logQuery(handoff, from) {
    const params = { from_row: from, limit: 2000 };
    if (handoff?.port) params.port = handoff.port;
    else if (Number.isInteger(handoff?.tab_id)) params.tab_id = handoff.tab_id;
    else params.tab_id = 0;
    return params;
  }

  async function snapshotLog(control, handoff) {
    let from = 0;
    let text = '';
    let dropped = 0;
    let next = 0;
    for (let page = 0; page < 10000; page += 1) {
      const data = payload(await call('log.lines.get', logQuery(handoff, from), control));
      dropped = Number(data.dropped ?? dropped);
      const lines = Array.isArray(data.lines) ? data.lines.map(String) : [];
      for (const line of lines) text += `${line.endsWith('\n') ? line : `${line}\n`}`;
      next = Number(data.next_row ?? from + lines.length);
      if (!lines.length || next <= from || lines.length < 2000) break;
      from = next;
    }
    return { bytes: Buffer.byteLength(text), next, dropped };
  }

  function isSerial(resource) {
    if(resource?.kind==='wiparse_dongle' && resource?.purpose==='i2c') return false;
    return SERIAL_KINDS.has(resource?.kind) || Boolean(resource?.port);
  }

  async function handoffSerial(resource, control) {
    const port = resource.port || resource.resource;
    if (!port) throw coded('SERIAL.PORT_REQUIRED', resource.kind || 'serial');
    const status = payload(await call('serial.monitor.status', {}, control));
    const existing = sessionFor(status, port);
    let started = false;
    let tab_id = existing?.tab_id;
    if (!existing) {
      const baud = resource.selector?.baud || resource.baud || 115200;
      const opened = payload(await call('serial.monitor.start', { port, baud }, control));
      started = opened.already_open !== true;
      tab_id = Number.isInteger(opened.tab_id) ? opened.tab_id : tab_id;
    }
    const handoff = { port, tab_id };
    let cursor = { bytes: 0, next: 0, dropped: 0 };
    try { cursor = await snapshotLog(control, handoff); } catch (error) {
      if (started) await call('serial.monitor.stop', { port }, control).catch(() => {});
      throw error;
    }
    return { port, started, cursor, tab_id };
  }

  function mapWaveformMetrics(detail) {
    const raw = detail?.metrics;
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) return undefined;
    const out = {};
    for (const [channel, metrics] of Object.entries(raw)) {
      if (!metrics || typeof metrics !== 'object' || Array.isArray(metrics)) continue;
      const row = {};
      for (const [metric, reading] of Object.entries(metrics)) {
        if (!reading || typeof reading !== 'object') continue;
        const value = Number(reading.value);
        const unit = reading.unit;
        const quality = reading.quality;
        if (!Number.isFinite(value) || typeof unit !== 'string' || !unit || quality !== 'good') continue;
        row[metric] = { value, unit, quality };
      }
      if (Object.keys(row).length) out[channel] = row;
    }
    return Object.keys(out).length ? out : undefined;
  }

  async function confirmedCall(method, params, control, match) {
    const ms = budgetMs(control);
    const operation = track(control, method);
    const captured = captureEvents(events);
    let accepted;
    try {
      accepted = await invoke(method, params, ms);
    } catch (error) {
      captured.cancel();
      const idle = answeredFailure(error);
      finish(operation, idle);
      error.quiescent = idle;
      throw error;
    }
    const jobId = accepted?.data?.job_id;
    if (jobId == null) {
      captured.cancel();
      finish(operation, false);
      throw coded('IO.UNCONFIRMED', `${method} did not return a job id`);
    }
    captured.arm(event => match(event, jobId));
    const pending = captured.promise.then(event => {
      finish(operation, true);
      return event;
    }, error => {
      finish(operation, false);
      throw error;
    });
    pending.catch(() => {});
    let timer;
    try {
      const event = await Promise.race([
        pending,
        new Promise((_, reject) => { timer = setTimeout(() => reject(coded('IO.TIMEOUT', method)), ms); }),
      ]);
      if (event.type === 'instrument.error' || event.data?.ok === false) {
        const error = coded('IO.INSTRUMENT', event.data?.detail?.error || event.data?.message || event.data?.detail?.message || method);
        error.quiescent = true;
        throw error;
      }
      return event;
    } finally {
      clearTimeout(timer);
    }
  }

  function isJob(event, jobId) {
    const id = event?.data?.job_id;
    if (Number(id) !== Number(jobId)) return false;
    return event.type === 'instrument.job_done' || event.type === 'instrument.error';
  }

  async function sourceOutput(resource, channel, enabled, extra) {
    const control = extra || controlFrom(arguments);
    const id = deviceIdOf(resource);
    await confirmedCall('instrument.command', {
      device_id: id,
      command: { SourceOutput: { channel: Number(channel), enabled: Boolean(enabled) } },
    }, control, isJob);
  }

  async function readMeasurement(resource, channel, control, expectedUnit) {
    const id = deviceIdOf(resource);
    const ms = budgetMs(control);
    const operation = track(control, 'instrument.measure');
    const captured = captureEvents(events);
    captured.arm(event => event.type === 'instrument.measurements' && Number(event.data?.device_id) === id);
    try {
      await invoke('instrument.measure', { device_id: id }, ms);
    } catch (error) {
      captured.cancel();
      const idle = answeredFailure(error);
      finish(operation, idle);
      error.quiescent = idle;
      throw error;
    }
    const pending = captured.promise.then(event => {
      finish(operation, true);
      return event;
    }, error => {
      finish(operation, false);
      throw error;
    });
    pending.catch(() => {});
    let timer;
    try {
      const event = await Promise.race([
        pending,
        new Promise((_, reject) => { timer = setTimeout(() => reject(coded('IO.TIMEOUT', 'instrument.measure')), ms); }),
      ]);
      const readings = Array.isArray(event.data?.readings) ? event.data.readings : [];
      const candidates=readings.filter(item=>channelMatches(item,channel) && (!expectedUnit || item?.unit===expectedUnit));
      const reading = candidates.length===1 ? candidates[0] : null;
      if (!reading || !Number.isFinite(Number(reading.value)) || !reading.unit) {
        const error = coded('MEASURE.EMPTY', 'instrument.measure returned no reading');
        error.quiescent = true;
        throw error;
      }
      return reading;
    } finally {
      clearTimeout(timer);
    }
  }

  async function placedFile(dir, folder, name, event) {
    const targetDir = path.join(dir, folder);
    fs.mkdirSync(targetDir, { recursive: true });
    const dest = path.join(targetDir, name);
    const reported = event?.data?.detail?.path;
    if (reported && path.resolve(reported) !== path.resolve(dest)) {
      if (!fs.existsSync(reported)) throw coded('EVIDENCE.MISSING_ARTIFACT', reported);
      fs.copyFileSync(reported, dest);
    }
    if (!fs.existsSync(dest)) throw coded('EVIDENCE.MISSING_ARTIFACT', dest);
    const bytes = fs.readFileSync(dest);
    return { path: dest, sha256: sha256(bytes) };
  }

  const adapter = {
    version: FPGA_HOST_VERSION,
    capabilities: [...FPGA_HOST_CAPABILITIES],
    devices,
    async preflight({ resources, identity, station } = {}) {
      const live = await devices();
      const missing = Object.entries(resources || {}).filter(([, resource]) => !live.some(device => sameDevice(device, resource))).map(([role]) => role);
      if (missing.length) throw coded('RESOURCE.MISSING', missing.join(','));
      const stationId = station?.station_id || identity?.station_id;
      if (!stationId) return;
      const file = lockPath(stationId);
      if (!fs.existsSync(file)) return;
      let lock = null;
      try { lock = JSON.parse(fs.readFileSync(file, 'utf8')); } catch { lock = null; }
      if (lock && pidAlive(lock.pid) && lock.pid !== process.pid) throw coded('STATION.LOCKED', `pid ${lock.pid}`);
    },
    async storage({ minimum_free_bytes } = {}) {
      if (!ctx.dataRoot) throw coded('STORAGE.UNKNOWN', 'data root is not set');
      const minimum = Number(minimum_free_bytes);
      if (!Number.isFinite(minimum) || minimum < 0) throw coded('STORAGE.MINIMUM_REQUIRED', 'minimum_free_bytes');
      const free = freeBytes(ctx.dataRoot);
      if (free < BigInt(Math.floor(minimum))) throw coded('STORAGE.LOW', `${free} bytes free, need ${Math.floor(minimum)}`);
      return { ok: true, free_bytes: free <= BigInt(Number.MAX_SAFE_INTEGER) ? Number(free) : Number.MAX_SAFE_INTEGER };
    },
    async acquire({ station_id, run_id, resources } = {}) {
      if (!station_id) throw coded('STATION.ID_REQUIRED', 'station_id');
      if(ctx.dataRoot && fs.existsSync(path.join(ctx.dataRoot,'.station-recovery.json')))throw coded('STATION.RECOVERY_REQUIRED','工位状态未知，需要人工确认');
      const file = lockPath(station_id);
      const flat = flattenResources(resources);
      claim(file, { pid: process.pid, station_id, run_id, resources: flat.map(resourceKey).filter(Boolean).sort(), at: new Date().toISOString() });
      const lease = { station_id, run_id, pid: process.pid, lock_path: file, resources: flat.map(resourceKey), devices:flat.filter(r=>r.device_id!=null).map(r=>({key:resourceKey(r),device_id:String(r.device_id),kind:r.kind})), i2c_devices:flat.filter(r=>['wiparse_dongle','usb_bridge','bridge'].includes(r.kind)&&r.device_id!=null).map(r=>({key:resourceKey(r),device_id:String(r.device_id),kind:r.kind})), cursors: {}, rows: {}, dropped: {}, handoffs: [] };
      try {
        for (const resource of flat) {
          if (!isSerial(resource)) continue;
          const handoff = await handoffSerial(resource, { timeout_s: 20 });
          const key = resourceKey(resource);
          lease.cursors[key] = handoff.cursor.bytes;
          lease.rows[key] = handoff.cursor.next;
          lease.dropped[key] = handoff.cursor.dropped;
          lease.handoffs.push({ key, port: handoff.port, started: handoff.started, tab_id: handoff.tab_id });
        }
      } catch (error) {
        try { if (JSON.parse(fs.readFileSync(file, 'utf8')).pid === process.pid) fs.unlinkSync(file); } catch { /* keep the original error */ }
        throw error;
      }
      activeLease = lease;
      return lease;
    },
    async release(lease) {
      if (!lease?.lock_path) throw coded('LEASE.REQUIRED', 'missing lease');
      let current = null;
      try { current = JSON.parse(fs.readFileSync(lease.lock_path, 'utf8')); } catch { throw coded('LEASE.MISSING', lease.lock_path); }
      if (current.pid !== process.pid) throw coded('LEASE.PID_MISMATCH', `lock pid ${current.pid}`);
      if (current.manual_recovery_required===true) throw coded('STATION.RECOVERY_REQUIRED');
      for (const handoff of lease.handoffs || []) {
        if (handoff.started) await call('serial.monitor.stop', { port: handoff.port }, { timeout_s: 15 });
      }
      fs.unlinkSync(lease.lock_path);
      if (activeLease === lease) activeLease = null;
    },
    sourceOutput,
    closeEvents() { if (!ctx.events) events.stop(); },
    async markLeaseUncertain(lease, reason='I2C_STATE_UNKNOWN') {
      if(!lease?.lock_path || activeLease!==lease)throw coded('LEASE.REQUIRED');
      const current=JSON.parse(fs.readFileSync(lease.lock_path,'utf8'));
      if(current.pid!==process.pid || current.run_id!==lease.run_id)throw coded('LEASE.LOST');
      if(!ctx.dataRoot)throw coded('STATION.RECOVERY_PATH_REQUIRED');
      const recovery=path.join(ctx.dataRoot,'.station-recovery.json');
      if(fs.existsSync(recovery))throw coded('STATION.RECOVERY_REQUIRED');
      // Survives runner cleanup and blocks projects using the same data root.
      atomicWrite(recovery,JSON.stringify({station_id:lease.station_id,run_id:lease.run_id,reason,at:new Date().toISOString(),manual_recovery_required:true}));
      atomicWrite(lease.lock_path,JSON.stringify({...current,manual_recovery_required:true,recovery_reason:reason}));
    },
    async assertI2cLease(resource) {
      if(!activeLease?.lock_path || !activeLease.i2c_devices?.some(d=>d.key===resourceKey(resource)&&d.device_id===String(resource?.device_id))) throw coded('LEASE.REQUIRED','I2C resource is not leased');
      let current;
      try { current=JSON.parse(fs.readFileSync(activeLease.lock_path,'utf8')); } catch { throw coded('LEASE.MISSING'); }
      if(current.pid!==process.pid || current.run_id!==activeLease.run_id || !current.resources?.includes(resourceKey(resource))) throw coded('LEASE.LOST');
      return true;
    },
    async assertInstrumentLease(resource) {
      if(!activeLease?.lock_path || !activeLease.devices?.some(d=>d.key===resourceKey(resource)&&d.device_id===String(resource?.device_id)))throw coded('LEASE.REQUIRED','instrument is not leased');
      let current;try{current=JSON.parse(fs.readFileSync(activeLease.lock_path,'utf8'));}catch{throw coded('LEASE.MISSING');}
      if(current.pid!==process.pid || current.run_id!==activeLease.run_id || !current.resources?.includes(resourceKey(resource)))throw coded('LEASE.LOST');
      return true;
    },
    async i2cCommand(request = {}) {
      await adapter.assertI2cLease(request.resource);
      const command=request.command;
      const kind=request.resource?.kind, allowed=kind==='wiparse_dongle'?['DongleI2cRead','DongleI2cWrite']:['usb_bridge','bridge'].includes(kind)?['BridgeI2c']:[];
      if(!command || typeof command!=='object' || Array.isArray(command) || Object.keys(command).length!==1 || !allowed.includes(Object.keys(command)[0])) throw coded('I2C.COMMAND_UNSUPPORTED');
      return confirmedCall('instrument.command',{device_id:deviceIdOf(request.resource),command},request,isJob);
    },
    async loadCommand(request = {}) {
      await adapter.assertInstrumentLease(request.resource);
      if(request.resource?.kind!=='electronic_load')throw coded('LOAD.RESOURCE_INVALID');
      const command=request.command;
      if(!command || typeof command!=='object' || Array.isArray(command) || Object.keys(command).length!==1)throw coded('LOAD.COMMAND_INVALID');
      if(Object.hasOwn(command,'LoadInput')) {
        if(typeof command.LoadInput!=='boolean')throw coded('LOAD.COMMAND_INVALID');
      } else if(Object.hasOwn(command,'LoadMode')) {
        if(command.LoadMode!=='CC')throw coded('LOAD.COMMAND_INVALID');
      } else if(Object.hasOwn(command,'LoadLevel')) {
        const level=command.LoadLevel;
        if(!level || level.mode!=='CC' || !Number.isFinite(level.value) || level.value<0)throw coded('LOAD.COMMAND_INVALID');
      } else throw coded('LOAD.COMMAND_INVALID');
      return confirmedCall('instrument.command',{device_id:deviceIdOf(request.resource),command},request,isJob);
    },
    async waitSerialSequence({resource,markers,active_marker,timeout_ms,stopped=()=>false}={}) {
      const lease=activeLease,key=resourceKey(resource),handoff=lease?.handoffs?.find(h=>h.key===key);
      if(!lease || !handoff)throw coded('LEASE.REQUIRED','serial resource not leased');
      if(!Array.isArray(markers)||!markers.length||markers.some(m=>typeof m!=='string'||!m.trim()||m.length>128))throw coded('LOG.MARKERS_INVALID');
      if(active_marker!=null && (typeof active_marker!=='string'||!active_marker.trim()||active_marker.length>128))throw coded('LOG.MARKERS_INVALID');
      if(!Number.isInteger(timeout_ms)||timeout_ms<1||timeout_ms>300000)throw coded('LOG.TIMEOUT_INVALID');
      const deadline=performance.now()+timeout_ms,found=[];
      // Protocol identifiers are tokens: ASK 31 must not match ASK 310 or ASK 31A.
      // Other user-defined markers retain their literal substring semantics.
      const matchesMarker=(line,marker)=>{
        if(!/^ASK\s+[0-9a-f]+$/i.test(marker))return line.includes(marker);
        const escaped=marker.replace(/[.*+?^${}()|[\]\\]/g,'\\$&');
        return new RegExp(`(^|[^A-Za-z0-9_])${escaped}(?![A-Za-z0-9_])`).test(line);
      };
      let row=lease.rows[key],dropped=lease.dropped[key],index=0;
      while(performance.now()<deadline) {
        if(stopped())throw coded('RUN.ABORTED');
        const remain=Math.max(1,Math.floor(deadline-performance.now()));
        const page=payload(await call('log.lines.get',logQuery(handoff,row),{timeout_s:Math.min(remain,2000)/1000}));
        const nowDropped=Number(page.dropped??dropped);
        if(!Number.isFinite(nowDropped)||nowDropped!==dropped)throw coded('LOG.DROPPED','无法确认协议顺序');
        dropped=nowDropped;
        if(!Array.isArray(page.lines))throw coded('LOG.RESPONSE_INVALID');
        const lines=page.lines.map(String);
        for(const line of lines) {
          if(active_marker && matchesMarker(line,active_marker)) {
            found.push({marker:active_marker,line:line.slice(0,1024),observed_mono_ms:performance.now(),time_source:'host_poll'});
            index=markers.length;break;
          }
          if(matchesMarker(line,markers[index])) {
            found.push({marker:markers[index],line:line.slice(0,1024),observed_mono_ms:performance.now(),time_source:'host_poll'});
            index++;
            if(index===markers.length)break;
          }
        }
        const next=Number(page.next_row??row+lines.length);
        if(!Number.isSafeInteger(next)||next<row)throw coded('LOG.CURSOR_INVALID');
        row=next;lease.rows[key]=row;lease.dropped[key]=dropped;
        if(index===markers.length)return {events:found,complete_raw:false};
        await new Promise(resolve=>setTimeout(resolve,Math.min(50,Math.max(1,deadline-performance.now()))));
      }
      throw coded('RX.READY_TIMEOUT','未观察到新的 RX 建链事件，请重新建立充电');
    },
    async voltage(resource, channel, extra) {
      const reading = await readMeasurement(resource, channel, extra || controlFrom(arguments),'V');
      return Number(reading.value);
    },
    async probeReset(resource, options = {}) {
      const control = options?.operation_id || options?.deadline_mono_ms ? options : controlFrom(arguments);
      await confirmedCall('instrument.command', {
        device_id: deviceIdOf(resource),
        command: { ProbeReset: { hardware: options?.hardware !== false } },
      }, control, isJob);
    },
    async collectLog(request = {}) {
      const lease = activeLease;
      if (!lease) throw coded('LEASE.REQUIRED', 'acquire before collectLog');
      const end = Number(request.end_mono_ms);
      if (!Number.isFinite(end)) throw coded('LOG.WINDOW_REQUIRED', 'end_mono_ms');
      const key = resourceKey(request.resource);
      const start = lease.cursors[key] ?? 0;
      let row = lease.rows[key] ?? 0;
      let droppedSeen = lease.dropped[key] ?? 0;
      const handoff = (lease.handoffs || []).find(item => item.key === key) || { port: request.resource?.port || request.resource?.resource };
      const chunks = [];
      const lines = [];
      let droppedLines = 0;
      const stopped = () => (typeof request.stopped === 'function' && request.stopped()) || request.signal?.aborted;
      let covered = Number.isFinite(Number(request.start_mono_ms)) ? performance.now() <= Number(request.start_mono_ms) + 50 : true;
      while (performance.now() < end) {
        if (stopped()) { covered = false; break; }
        let data;
        try { data = payload(await call('log.lines.get', logQuery(handoff, row), request)); }
        catch (error) {
          covered = false;
          break;
        }
        const dropped = Number(data.dropped ?? droppedSeen);
        if (dropped > droppedSeen) { droppedLines += dropped - droppedSeen; covered = false; }
        droppedSeen = dropped;
        const page = Array.isArray(data.lines) ? data.lines.map(String) : [];
        for (const text of page) {
          const piece = text.endsWith('\n') ? text : `${text}\n`;
          chunks.push(piece);
          lines.push({ mono_ms: performance.now(), utc: new Date().toISOString(), text });
        }
        const next = Number(data.next_row ?? row + page.length);
        if (next > row) row = next;
        if (Number.isFinite(end) && performance.now() >= end) break;
        const delay = Math.min(50, Math.max(1, (Number.isFinite(end) ? end : performance.now() + 50) - performance.now()));
        await new Promise(resolve => {
          const timer = setTimeout(resolve, delay);
          if (request.signal) request.signal.addEventListener('abort', () => { clearTimeout(timer); resolve(); }, { once: true });
        });
      }
      if (stopped() || (Number.isFinite(end) && performance.now() < end)) covered = false;
      const raw = Buffer.from(chunks.join(''), 'utf8');
      const endCursor = start + raw.length;
      lease.cursors[key] = endCursor;
      lease.rows[key] = row;
      lease.dropped[key] = droppedSeen;
      // log.lines.get supplies decoded lines, not the received byte stream or
      // device timestamps. Preserve the text for diagnosis but never certify it
      // as complete raw evidence, even when every polled line was returned.
      return { raw_base64: raw.toString('base64'), lines, start_cursor: start, end_cursor: endCursor, dropped_lines: droppedLines, window_covered: covered, complete: false, provenance:'reconstructed_text', timestamp_source:'host_poll', quality_issues:['LOG.RAW_BYTES_UNAVAILABLE','LOG.EVENT_TIME_UNAVAILABLE'] };
    },
    async capture(request = {}) {
      const destName = safeStem(request.config?.id, '.png');
      const shots = path.join(request.dir, 'shots');
      fs.mkdirSync(shots, { recursive: true });
      const dest = path.join(shots, destName);
      const event = await confirmedCall('instrument.capture', { device_id: deviceIdOf(request.resource), path: dest, wait: true }, request, isJob);
      return { screenshot: await placedFile(request.dir, 'shots', destName, event) };
    },
    async waveformSource(request = {}) {
      const destName = safeStem(request.config?.id, '.isf');
      const waves = path.join(request.dir, 'waves');
      fs.mkdirSync(waves, { recursive: true });
      const event = await confirmedCall('instrument.waveform_source', {
        device_id: deviceIdOf(request.resource),
        dir: waves,
        filename: destName,
        overwrite: true,
      }, request, isJob);
      const artifact = await placedFile(request.dir, 'waves', destName, event);
      const metrics = mapWaveformMetrics(event?.data?.detail);
      return metrics ? { artifact, metrics } : { artifact };
    },
    async cleanup(request = {}) {
      const state = request.safe_power_state;
      if (state == null) return { ok: true };
      const resources = request.resources || {};
      const targets = [];
      if (state === 'off' || state === false || state === 'disabled') {
        for (const resource of Object.values(resources)) {
          if (resource?.kind === 'dc_source') targets.push({ resource, channel: resource.selector?.channel ?? resource.channel ?? 1 });
        }
      } else if (state && typeof state === 'object') {
        for (const [role, spec] of Object.entries(state)) {
          if (resources[role]) targets.push({ resource: resources[role], channel: spec?.channel ?? 1 });
        }
      } else throw coded('CLEANUP.UNSUPPORTED', String(state));
      for (const target of targets) await sourceOutput(target.resource, target.channel, false, request);
      return { ok: true };
    },
    async cancelAndConfirm({ operation_id } = {}) {
      if (operation_id && !inflight.has(operation_id)) {
        if (completed.get(operation_id) === false) throw coded('IO.UNCONFIRMED', operation_id);
        return { quiescent: true };
      }
      const targets = operation_id ? [inflight.get(operation_id)].filter(Boolean) : [...inflight.values()];
      const results = await Promise.all(targets.map(operation => operation.settled));
      if (results.some(result => !result.confirmed)) throw coded('IO.UNCONFIRMED', operation_id || 'instrument');
      return { quiescent: true };
    },
    async measure(request = {}) {
      const resource = request.resources?.[request.rule?.resource_role] ?? request.resource;
      const reading = await readMeasurement(resource, request.rule?.channel ?? request.channel, request,request.rule?.unit ?? request.unit);
      return { value: Number(reading.value), unit: String(reading.unit), quality: 'good' };
    },
  };
  return adapter;
}

export function attachHostAdapter(ctx) {
  if (ctx?.plugin?.host_adapter === FPGA_HOST_VERSION) ctx.fpgaAdapter = createFpgaHostAdapter(ctx);
  return ctx;
}
