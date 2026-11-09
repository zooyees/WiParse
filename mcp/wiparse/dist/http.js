/** Direct HTTP client for WiParse GUI embedded API (C+E). */
import { execFile } from 'node:child_process';
import { existsSync } from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
export function defaultApiUrl() {
    return (process.env.WIPARSE_URL ?? "http://127.0.0.1:7878").replace(/\/$/, "");
}
async function readJson(res) {
    const reader = res.body?.getReader();
    if (!reader)
        throw Error('API response has no body');
    const chunks = [];
    let size = 0;
    while (true) {
        const { done, value } = await reader.read();
        if (done)
            break;
        size += value.byteLength;
        if (size > 16 * 1024 * 1024) {
            await reader.cancel();
            throw Error('API response exceeds 16 MiB; operation result unknown');
        }
        chunks.push(value);
    }
    const text = Buffer.concat(chunks).toString('utf8');
    try {
        return JSON.parse(text);
    }
    catch {
        throw new Error(`invalid API JSON (HTTP ${res.status}); operation result unknown`);
    }
}
export function isLocalBridge(baseUrl) {
    try {
        const url = new URL(baseUrl);
        return url.protocol === 'http:' && url.hostname === '127.0.0.1' && url.port === '7878' && !url.username && !url.password && (url.pathname === '' || url.pathname === '/') && !url.search && !url.hash;
    }
    catch {
        return false;
    }
}
export function localHeaders(baseUrl, env = process.env) {
    const token = env.WIPARSE_LOCAL_API_TOKEN;
    return isLocalBridge(baseUrl) && token && /^[0-9a-f]{8}(?:-[0-9a-f]{4}){3}-[0-9a-f]{12}$/i.test(token) ? { Authorization: `Bearer ${token}` } : {};
}
function cliInvoke(method, params, baseUrl, timeoutMs) {
    const root = fileURLToPath(new URL('../../../', import.meta.url));
    const candidates = process.env.WIPARSE_CLI ? [path.resolve(process.env.WIPARSE_CLI)] : [path.join(root, 'bin', 'WiParse-CLI.exe'), path.join(root, 'WiParse-CLI.exe'), path.join(root, 'target', 'release', 'wiparse.exe')];
    const cli = candidates.find(candidate => existsSync(candidate));
    if (!cli)
        return Promise.reject(Error('Protected host requires the matching WiParse CLI beside the application, or WIPARSE_CLI'));
    return new Promise((resolve, reject) => {
        execFile(cli, ['api', 'invoke', '--method', method, '--params', JSON.stringify(params)], { windowsHide: true, shell: false, timeout: timeoutMs, maxBuffer: 16 * 1024 * 1024, env: { ...process.env, WIPARSE_URL: baseUrl } }, (error, stdout) => {
            try {
                const body = JSON.parse(stdout);
                if (typeof body.ok !== 'boolean')
                    throw Error('Invalid CLI envelope');
                resolve(body);
            }
            catch {
                reject(Error(error ? 'Credential-aware CLI failed; operation result may be unknown. No retry was performed.' : 'CLI returned invalid JSON; operation result unknown. No retry was performed.'));
            }
        });
    });
}
export async function apiHealth(baseUrl = defaultApiUrl()) {
    const res = await fetch(`${baseUrl}/v1/health`, { redirect: 'error', signal: AbortSignal.timeout(5000) });
    const body = (await readJson(res));
    return body;
}
export async function apiCapabilities(baseUrl = defaultApiUrl()) {
    const res = await fetch(`${baseUrl}/v1/capabilities`, { redirect: 'error', signal: AbortSignal.timeout(5000) });
    return (await readJson(res));
}
export async function apiInvoke(method, params = {}, baseUrl = defaultApiUrl(), timeoutMs = 90_000) {
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), timeoutMs);
    try {
        const res = await fetch(`${baseUrl}/v1/invoke`, {
            method: "POST",
            headers: { "Content-Type": "application/json", ...localHeaders(baseUrl) },
            body: JSON.stringify({ method, params }),
            signal: ctrl.signal,
            redirect: 'error',
        });
        // Only a positively rejected request may be submitted through the CLI.
        // It decrypts the current-user instance credential; transport failures and
        // HTTP 5xx must never replay a possibly executed instrument operation.
        if (res.status === 401 && isLocalBridge(baseUrl)) {
            await res.body?.cancel();
            clearTimeout(timer);
            return await cliInvoke(method, params, baseUrl, timeoutMs);
        }
        if (res.status >= 500) {
            await res.body?.cancel();
            throw Error(`HTTP ${res.status}; operation result unknown. No retry was performed.`);
        }
        return (await readJson(res));
    }
    finally {
        clearTimeout(timer);
    }
}
