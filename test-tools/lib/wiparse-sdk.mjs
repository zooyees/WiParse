/**
 * WiParse helpers for test-tool plugins: CLI + HTTP API.
 */

import { spawn } from "node:child_process";
import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

/** Cap captured CLI output to avoid unbounded memory on runaway logs. */
const MAX_CAPTURE_CHARS = 512 * 1024;
/** Default CLI process timeout (ms). */
const DEFAULT_CLI_TIMEOUT_MS = 120_000;

export function resolveCliPath(explicit) {
  if (explicit && String(explicit).trim()) {
    return String(explicit).trim();
  }
  if (process.env.WIPARSE_CLI) {
    return process.env.WIPARSE_CLI;
  }
  const roots = [
    path.resolve(__dirname, "..", "..", "dist"),
    path.resolve(__dirname, "..", "dist"),
    process.cwd(),
  ];
  const names =
    process.platform === "win32"
      ? ["WiParse-CLI.exe", "wiparse-cli.exe", "wiparse.exe"]
      : ["wiparse-cli", "wiparse"];
  for (const root of roots) {
    for (const name of names) {
      const p = path.join(root, name);
      if (fs.existsSync(p)) return p;
    }
  }
  return process.platform === "win32" ? "WiParse-CLI.exe" : "wiparse";
}

function appendCapped(buf, chunk, max) {
  if (buf.length >= max) return buf;
  const next = buf + chunk;
  if (next.length <= max) return next;
  return next.slice(0, max) + "\n…[truncated]…\n";
}

function runProcess(cliPath, args, { cwd, env, log, timeoutMs = DEFAULT_CLI_TIMEOUT_MS } = {}) {
  return new Promise((resolve, reject) => {
    const child = spawn(cliPath, args, {
      cwd,
      env: { ...process.env, ...env },
      windowsHide: true,
      shell: false,
    });
    let stdout = "";
    let stderr = "";
    let settled = false;
    const timer =
      timeoutMs > 0
        ? setTimeout(() => {
            if (settled) return;
            try {
              child.kill();
            } catch {
              /* ignore */
            }
            settled = true;
            reject(new Error(`wiparse CLI timeout after ${timeoutMs}ms`));
          }, timeoutMs)
        : null;

    child.stdout?.on("data", (buf) => {
      const s = buf.toString();
      stdout = appendCapped(stdout, s, MAX_CAPTURE_CHARS);
      log?.("stdout", s);
    });
    child.stderr?.on("data", (buf) => {
      const s = buf.toString();
      stderr = appendCapped(stderr, s, MAX_CAPTURE_CHARS);
      log?.("stderr", s);
    });
    child.on("error", (err) => {
      if (timer) clearTimeout(timer);
      if (settled) return;
      settled = true;
      reject(err);
    });
    child.on("close", (code) => {
      if (timer) clearTimeout(timer);
      if (settled) return;
      settled = true;
      resolve({ code: code ?? 1, stdout, stderr });
    });
  });
}

/**
 * HTTP client against running WiParse.exe (`/v1/health`, `/v1/invoke`).
 * @param {{ url?: string, log?: Function }} [opts]
 */
export function createHttpClient(opts = {}) {
  const base = String(opts.url || process.env.WIPARSE_URL || "http://127.0.0.1:7878").replace(
    /\/$/,
    ""
  );
  const log = opts.log;

  async function health(ms = 15_000) {
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), ms);
    try {
      const res = await fetch(`${base}/v1/health`, { signal: ctrl.signal });
      if (!res.ok) {
        throw new Error(`HTTP ${res.status} /v1/health`);
      }
      return res.json();
    } finally {
      clearTimeout(timer);
    }
  }

  async function invoke(method, params = {}, ms = 120_000, opts = {}) {
    const allowFail = Boolean(opts.allowFail);
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), ms);
    try {
      const res = await fetch(`${base}/v1/invoke`, {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify({ method, params }),
        signal: ctrl.signal,
      });
      if (!res.ok) {
        const body = await res.text().catch(() => "");
        const msg = `HTTP ${res.status} ${method}: ${body.slice(0, 240)}`;
        log?.("stderr", `${msg}\n`);
        if (allowFail) return { ok: false, error: msg };
        throw new Error(msg);
      }
      const json = await res.json();
      if (!allowFail && json && json.ok === false) {
        const err = json.error || json.message || JSON.stringify(json);
        throw new Error(`${method} failed: ${err}`);
      }
      return json;
    } finally {
      clearTimeout(timer);
    }
  }

  return { url: base, health, invoke };
}

/**
 * @param {{ cliPath?: string, url?: string, log?: (stream: string, text: string) => void, cwd?: string }} [opts]
 */
export function createClient(opts = {}) {
  const cliPath = resolveCliPath(opts.cliPath);
  const url = opts.url || process.env.WIPARSE_URL || "http://127.0.0.1:7878";
  const log = opts.log;
  const cwd = opts.cwd;
  const http = createHttpClient({ url, log });

  async function cli(args, extraEnv = {}, timeoutMs = DEFAULT_CLI_TIMEOUT_MS) {
    const list = Array.isArray(args) ? args.map(String) : [];
    log?.("info", `$ ${cliPath} ${list.join(" ")}\n`);
    const result = await runProcess(cliPath, list, {
      cwd,
      env: { WIPARSE_URL: url, ...extraEnv },
      log,
      timeoutMs,
    });
    if (result.code !== 0) {
      const err = new Error(
        `wiparse exited ${result.code}: ${(result.stderr || result.stdout).trim()}`
      );
      err.result = result;
      throw err;
    }
    return result;
  }

  return {
    cliPath,
    url,
    cli,
    http,
    api: {
      /** Prefer HTTP health (fast); falls back to CLI. */
      health: async () => {
        try {
          return await http.health();
        } catch {
          return cli(["api", "health"], {}, 30_000);
        }
      },
      /** Prefer HTTP invoke for long-running GUI ops. */
      invoke: async (method, params = {}, ms) => {
        try {
          return await http.invoke(method, params, ms);
        } catch {
          return cli(
            [
              "api",
              "invoke",
              "--method",
              method,
              "--params",
              JSON.stringify(params),
            ],
            {},
            typeof ms === "number" ? ms : DEFAULT_CLI_TIMEOUT_MS
          );
        }
      },
    },
    serial: {
      select: ({ port, baud } = {}) => {
        const a = ["serial", "select"];
        if (port) a.push("--port", String(port));
        if (baud != null) a.push("--baud", String(baud));
        return cli(a);
      },
      start: ({ port, baud } = {}) => {
        const a = ["serial", "start"];
        if (port) a.push("--port", String(port));
        if (baud != null) a.push("--baud", String(baud));
        return cli(a);
      },
      send: ({ port, hex, text } = {}) => {
        const a = ["serial", "send"];
        if (port) a.push("--port", String(port));
        if (hex) a.push("--hex", String(hex));
        if (text) a.push("--text", String(text));
        return cli(a);
      },
    },
    ui: {
      show: (tab) => cli(["ui", "show", "--tab", String(tab)]),
      state: () => cli(["ui", "state"]),
    },
    test: {
      run: ({ plan, port, baud } = {}) => {
        const a = ["test", "run"];
        if (plan) a.push("--plan", String(plan));
        if (port) a.push("--port", String(port));
        if (baud != null) a.push("--baud", String(baud));
        return cli(a);
      },
      status: () => cli(["test", "status"]),
      pack: () => cli(["test", "pack"]),
    },
  };
}

export default { createClient, createHttpClient, resolveCliPath };
