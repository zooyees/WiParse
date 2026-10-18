/** Direct HTTP client for WiParse GUI embedded API (C+E). */
export function defaultApiUrl() {
    return (process.env.WIPARSE_URL ?? "http://127.0.0.1:7878").replace(/\/$/, "");
}
async function readJson(res) {
    const text = await res.text();
    try {
        return JSON.parse(text);
    }
    catch {
        throw new Error(`invalid API JSON (HTTP ${res.status}): ${text.slice(0, 200)}`);
    }
}
export async function apiHealth(baseUrl = defaultApiUrl()) {
    const res = await fetch(`${baseUrl}/v1/health`);
    const body = (await readJson(res));
    return body;
}
export async function apiCapabilities(baseUrl = defaultApiUrl()) {
    const res = await fetch(`${baseUrl}/v1/capabilities`);
    return (await readJson(res));
}
export async function apiInvoke(method, params = {}, baseUrl = defaultApiUrl(), timeoutMs = 90_000) {
    const ctrl = new AbortController();
    const timer = setTimeout(() => ctrl.abort(), timeoutMs);
    try {
        const res = await fetch(`${baseUrl}/v1/invoke`, {
            method: "POST",
            headers: { "Content-Type": "application/json" },
            body: JSON.stringify({ method, params }),
            signal: ctrl.signal,
        });
        return (await readJson(res));
    }
    finally {
        clearTimeout(timer);
    }
}
