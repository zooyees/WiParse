/**
 * TCP transport for WIPARSE-IF channels (UART :1234, I2C :1236, DAP USB/IP :3240).
 * TCP_NODELAY + KEEPALIVE as required by the ICD.
 */

import net from "node:net";
import { hostCandidates } from "./wiparse-if.mjs";

export function connectTcp({
  host,
  port,
  timeoutMs = 4000,
  noDelay = true,
  keepAlive = true,
  fallbackHost,
} = {}) {
  const candidates = hostCandidates(host, fallbackHost);
  const p = Number(port);
  if (!Number.isInteger(p) || p < 1 || p > 65535) {
    return Promise.reject(new Error(`invalid TCP port: ${port}`));
  }
  let lastErr = null;
  return (async () => {
    for (const h of candidates) {
      try {
        const sock = await connectOnce(h, p, timeoutMs, noDelay, keepAlive);
        sock.peer = { host: h, port: p };
        return sock;
      } catch (e) {
        lastErr = e;
      }
    }
    throw lastErr || new Error(`TCP connect failed ${candidates.join("|")}:${p}`);
  })();
}

function connectOnce(host, port, timeoutMs, noDelay, keepAlive) {
  return new Promise((resolve, reject) => {
    const sock = net.connect({ host, port });
    let settled = false;
    const timer = setTimeout(() => {
      if (settled) return;
      settled = true;
      sock.destroy();
      reject(new Error(`TCP timeout ${host}:${port} after ${timeoutMs}ms`));
    }, Math.max(200, Number(timeoutMs) || 4000));
    sock.once("connect", () => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      try {
        if (noDelay) sock.setNoDelay(true);
        if (keepAlive) sock.setKeepAlive(true, 10_000);
      } catch {
        /* ignore */
      }
      resolve(sock);
    });
    sock.once("error", (err) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      reject(err);
    });
  });
}

export function closeSocket(sock) {
  if (!sock) return;
  try {
    sock.destroy();
  } catch {
    /* ignore */
  }
}

export function writeAll(sock, buf) {
  return new Promise((resolve, reject) => {
    const data = Buffer.isBuffer(buf) ? buf : Buffer.from(buf);
    sock.write(data, (err) => (err ? reject(err) : resolve(data.length)));
  });
}

export function readSome(sock, { max = 4096, timeoutMs = 1000, min = 1 } = {}) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    let n = 0;
    let settled = false;
    const done = (err) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      sock.off("data", onData);
      sock.off("error", onErr);
      sock.off("close", onClose);
      sock.off("end", onEnd);
      if (err && n === 0) reject(err);
      else resolve(Buffer.concat(chunks, n));
    };
    const timer = setTimeout(() => {
      if (n >= min) done();
      else done(new Error("TCP read timeout"));
    }, Math.max(1, Number(timeoutMs) || 1000));
    const onData = (c) => {
      chunks.push(c);
      n += c.length;
      if (n >= max) done();
    };
    const onErr = (e) => done(e);
    const onClose = () => done(n ? undefined : new Error("TCP closed"));
    const onEnd = () => done();
    sock.on("data", onData);
    sock.once("error", onErr);
    sock.once("close", onClose);
    sock.once("end", onEnd);
  });
}

/**
 * Read one line ending in \n or \r. Ignores empty lines if skipEmpty.
 */
export function readLine(sock, { timeoutMs = 8000, max = 1024, skipEmpty = true } = {}) {
  return new Promise((resolve, reject) => {
    let buf = Buffer.alloc(0);
    let settled = false;
    const finish = (err, line) => {
      if (settled) return;
      settled = true;
      clearTimeout(timer);
      sock.off("data", onData);
      sock.off("error", onErr);
      sock.off("close", onClose);
      if (err) reject(err);
      else resolve(line);
    };
    const timer = setTimeout(() => finish(new Error("line timeout")), Number(timeoutMs) || 8000);
    const onData = (chunk) => {
      buf = Buffer.concat([buf, chunk]);
      if (buf.length > max) {
        finish(new Error("line overflow"));
        return;
      }
      const s = buf.toString("utf8");
      const m = s.match(/^([^\r\n]*)(\r\n|\n|\r)/);
      if (!m) return;
      const line = m[1];
      const rest = s.slice(m[0].length);
      if (rest) sock.unshift(Buffer.from(rest, "utf8"));
      if (skipEmpty && !line.trim()) {
        buf = Buffer.alloc(0);
        return;
      }
      finish(null, line);
    };
    const onErr = (e) => finish(e);
    const onClose = () => finish(new Error("TCP closed before line"));
    sock.on("data", onData);
    sock.once("error", onErr);
    sock.once("close", onClose);
  });
}

export async function readHelpLines(sock, { timeoutMs = 1500 } = {}) {
  const lines = [];
  const deadline = Date.now() + Math.max(200, Number(timeoutMs) || 1500);
  while (Date.now() < deadline) {
    try {
      const line = await readLine(sock, {
        timeoutMs: Math.max(50, deadline - Date.now()),
        skipEmpty: true,
      });
      lines.push(line);
      if (lines.length >= 1 && !String(line).toUpperCase().startsWith("OK ")) break;
    } catch {
      break;
    }
  }
  return lines;
}

export async function i2cRpc(sock, line, { timeoutMs = 8000 } = {}) {
  const body = line.endsWith("\n") ? line : `${line}\n`;
  await writeAll(sock, body);
  const cmd = String(line).trim().split(/\s+/)[0]?.toUpperCase();
  if (cmd === "HELP" || cmd === "?") {
    const lines = await readHelpLines(sock, { timeoutMs: Math.min(timeoutMs, 2000) });
    return { lines, text: lines.join("\n") };
  }
  const reply = await readLine(sock, { timeoutMs });
  return { lines: [reply], text: reply };
}

export function pumpUntil({
  sock,
  stop,
  durationMs = 0,
  onData,
  onTick,
} = {}) {
  return new Promise((resolve, reject) => {
    const started = Date.now();
    let bytes = 0;
    let settled = false;
    const finish = (err) => {
      if (settled) return;
      settled = true;
      clearInterval(tick);
      sock.off("data", onChunk);
      sock.off("error", onErr);
      sock.off("close", onClose);
      if (err) reject(err);
      else resolve({ bytes, elapsed_ms: Date.now() - started });
    };
    const onChunk = (c) => {
      bytes += c.length;
      try {
        onData?.(c);
      } catch (e) {
        finish(e);
      }
      if (stop?.()) finish();
    };
    const onErr = (e) => finish(e);
    const onClose = () => finish();
    sock.on("data", onChunk);
    sock.once("error", onErr);
    sock.once("close", onClose);
    const tick = setInterval(() => {
      const elapsed = Date.now() - started;
      onTick?.({ elapsed_ms: elapsed, bytes });
      if (stop?.()) finish();
      else if (durationMs > 0 && elapsed >= durationMs) finish();
    }, 200);
    if (stop?.()) finish();
  });
}

export default { connectTcp, closeSocket, writeAll, readSome, readLine, i2cRpc, pumpUntil };
