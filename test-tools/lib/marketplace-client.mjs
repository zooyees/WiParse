/**
 * HTTPS client for the Testing Hub marketplace cloud API.
 */

import {
  MarketplaceError,
  MARKETPLACE_ERROR,
  assertHttpsUrl,
  allowHttpFromEnv,
  normalizeTrustPolicy,
} from "./marketplace-trust.mjs";

export function createMarketplaceClient({
  baseUrl,
  token = null,
  trust = null,
  allowHttp = null,
  fetchImpl = globalThis.fetch,
} = {}) {
  if (!baseUrl || !String(baseUrl).trim()) {
    throw new MarketplaceError(MARKETPLACE_ERROR.CONFIG, "marketplace baseUrl required");
  }
  const httpOk = allowHttp == null ? allowHttpFromEnv() : Boolean(allowHttp);
  const root = assertHttpsUrl(baseUrl, { allowHttp: httpOk });
  const base = root.href.replace(/\/+$/, "");
  const policy = normalizeTrustPolicy(trust || {});

  async function request(method, p, { body = null, headers = {}, raw = false } = {}) {
    const url = p.startsWith("http") ? p : `${base}${p.startsWith("/") ? p : `/${p}`}`;
    assertHttpsUrl(url, { allowHttp: httpOk });
    const h = { Accept: "application/json", ...headers };
    if (token) h.Authorization = `Bearer ${token}`;
    let payload = body;
    if (body && typeof body === "object" && !(body instanceof Buffer) && !(body instanceof Uint8Array)) {
      h["Content-Type"] = h["Content-Type"] || "application/json";
      payload = JSON.stringify(body);
    }
    let res;
    try {
      res = await fetchImpl(url, { method, headers: h, body: payload });
    } catch (e) {
      throw new MarketplaceError(MARKETPLACE_ERROR.NETWORK, String(e?.message || e));
    }
    if (raw) return res;
    const text = await res.text();
    let data = null;
    try {
      data = text ? JSON.parse(text) : null;
    } catch {
      data = { raw: text };
    }
    if (!res.ok) {
      const code = data?.error?.code || (res.status === 401 || res.status === 403
        ? MARKETPLACE_ERROR.AUTH
        : res.status === 404
          ? MARKETPLACE_ERROR.NOT_FOUND
          : MARKETPLACE_ERROR.NETWORK);
      const msg = data?.error?.message || `HTTP ${res.status}`;
      throw new MarketplaceError(code, msg, { status: res.status, data });
    }
    return data;
  }

  return {
    baseUrl: base,
    trust: policy,

    health() {
      return request("GET", "/v1/health");
    },

    catalog(query = {}) {
      const qs = new URLSearchParams();
      for (const [k, v] of Object.entries(query)) {
        if (v != null && v !== "") qs.set(k, String(v));
      }
      const q = qs.toString();
      return request("GET", `/v1/catalog${q ? `?${q}` : ""}`);
    },

    getPlugin(id) {
      return request("GET", `/v1/plugins/${encodeURIComponent(id)}`);
    },

    getVersion(id, version) {
      return request(
        "GET",
        `/v1/plugins/${encodeURIComponent(id)}/versions/${encodeURIComponent(version)}`
      );
    },

    async download(id, version) {
      const res = await request(
        "GET",
        `/v1/plugins/${encodeURIComponent(id)}/versions/${encodeURIComponent(version)}/download`,
        { raw: true }
      );
      if (!res.ok) {
        const text = await res.text();
        let data = null;
        try {
          data = JSON.parse(text);
        } catch {
          /* ignore */
        }
        throw new MarketplaceError(
          res.status === 404 ? MARKETPLACE_ERROR.NOT_FOUND : MARKETPLACE_ERROR.NETWORK,
          data?.error?.message || `download failed HTTP ${res.status}`
        );
      }
      const ab = await res.arrayBuffer();
      return Buffer.from(ab);
    },

    /**
     * Publish: JSON meta + base64 artifact (simple for industrial CI without multipart deps).
     */
    publish(id, { meta, artifactBase64 }) {
      return request("POST", `/v1/plugins/${encodeURIComponent(id)}/versions`, {
        body: { meta, artifact_base64: artifactBase64 },
      });
    },

    unpublish(id, version) {
      return request(
        "DELETE",
        `/v1/plugins/${encodeURIComponent(id)}/versions/${encodeURIComponent(version)}`
      );
    },
  };
}

export default { createMarketplaceClient };
