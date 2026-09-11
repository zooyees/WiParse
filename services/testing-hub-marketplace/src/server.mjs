/**
 * HTTP API for Testing Hub plugin marketplace.
 */

import http from "node:http";
import { createStore } from "./store.mjs";

const SERVICE_VERSION = "0.1.0";

export function createApp({ dataDir, publishTokens = [] } = {}) {
  const store = createStore(dataDir);
  store.ensure();
  const tokens = new Set(
    (publishTokens || []).map((t) => String(t).trim()).filter(Boolean)
  );

  function unauthorized(res) {
    return sendJson(res, 401, {
      ok: false,
      error: { code: "E_AUTH", message: "unauthorized" },
    });
  }

  function requireAuth(req, res) {
    if (!tokens.size) {
      // Fail closed when no tokens configured for mutating routes.
      unauthorized(res);
      return false;
    }
    const h = req.headers.authorization || "";
    const m = String(h).match(/^Bearer\s+(.+)$/i);
    if (!m || !tokens.has(m[1].trim())) {
      unauthorized(res);
      return false;
    }
    return true;
  }

  async function handler(req, res) {
    try {
      const url = new URL(req.url || "/", `http://${req.headers.host || "localhost"}`);
      const { pathname } = url;
      const method = req.method || "GET";

      if (method === "GET" && pathname === "/v1/health") {
        return sendJson(res, 200, {
          ok: true,
          service: "testing-hub-marketplace",
          version: SERVICE_VERSION,
        });
      }

      if (method === "GET" && pathname === "/v1/catalog") {
        const plugins = store.listCatalog({
          channel: url.searchParams.get("channel") || undefined,
          type: url.searchParams.get("type") || undefined,
          q: url.searchParams.get("q") || undefined,
        });
        return sendJson(res, 200, { ok: true, plugins });
      }

      let m;
      if (method === "GET" && (m = pathname.match(/^\/v1\/plugins\/([^/]+)$/))) {
        const id = decodeURIComponent(m[1]);
        const plug = store.getPlugin(id);
        if (!plug) {
          return sendJson(res, 404, {
            ok: false,
            error: { code: "E_NOT_FOUND", message: `plugin not found: ${id}` },
          });
        }
        return sendJson(res, 200, { ok: true, plugin: plug });
      }

      if (
        method === "GET" &&
        (m = pathname.match(/^\/v1\/plugins\/([^/]+)\/versions\/([^/]+)$/))
      ) {
        const id = decodeURIComponent(m[1]);
        const version = decodeURIComponent(m[2]);
        const ver = store.getVersion(id, version);
        if (!ver) {
          return sendJson(res, 404, {
            ok: false,
            error: {
              code: "E_NOT_FOUND",
              message: `version not found: ${id}@${version}`,
            },
          });
        }
        return sendJson(res, 200, { ok: true, version: ver });
      }

      if (
        method === "GET" &&
        (m = pathname.match(/^\/v1\/plugins\/([^/]+)\/versions\/([^/]+)\/download$/))
      ) {
        const id = decodeURIComponent(m[1]);
        const version = decodeURIComponent(m[2]);
        const bytes = store.readArtifact(id, version);
        if (!bytes) {
          return sendJson(res, 404, {
            ok: false,
            error: {
              code: "E_NOT_FOUND",
              message: `artifact not found: ${id}@${version}`,
            },
          });
        }
        res.writeHead(200, {
          "Content-Type": "application/zip",
          "Content-Length": bytes.length,
          "Content-Disposition": `attachment; filename="${id}-${version}.zip"`,
        });
        res.end(bytes);
        return;
      }

      if (
        method === "POST" &&
        (m = pathname.match(/^\/v1\/plugins\/([^/]+)\/versions$/))
      ) {
        if (!requireAuth(req, res)) return;
        const id = decodeURIComponent(m[1]);
        const body = await readJson(req);
        const meta = body?.meta;
        const b64 = body?.artifact_base64;
        if (!meta || !b64) {
          return sendJson(res, 400, {
            ok: false,
            error: {
              code: "E_LAYOUT",
              message: "body requires meta and artifact_base64",
            },
          });
        }
        if (String(meta.id) !== id) {
          return sendJson(res, 400, {
            ok: false,
            error: { code: "E_LAYOUT", message: "meta.id must match path id" },
          });
        }
        let zip;
        try {
          zip = Buffer.from(String(b64), "base64");
        } catch {
          return sendJson(res, 400, {
            ok: false,
            error: { code: "E_LAYOUT", message: "invalid artifact_base64" },
          });
        }
        try {
          const stored = store.putVersion(meta, zip);
          return sendJson(res, 201, { ok: true, version: stored });
        } catch (e) {
          const code = e.code || "E_LAYOUT";
          const status = code === "E_HASH" ? 400 : 400;
          return sendJson(res, status, {
            ok: false,
            error: { code, message: e.message },
          });
        }
      }

      if (
        method === "DELETE" &&
        (m = pathname.match(/^\/v1\/plugins\/([^/]+)\/versions\/([^/]+)$/))
      ) {
        if (!requireAuth(req, res)) return;
        const id = decodeURIComponent(m[1]);
        const version = decodeURIComponent(m[2]);
        const ok = store.deleteVersion(id, version);
        if (!ok) {
          return sendJson(res, 404, {
            ok: false,
            error: {
              code: "E_NOT_FOUND",
              message: `version not found: ${id}@${version}`,
            },
          });
        }
        return sendJson(res, 200, { ok: true, id, version });
      }

      return sendJson(res, 404, {
        ok: false,
        error: { code: "E_NOT_FOUND", message: `no route ${method} ${pathname}` },
      });
    } catch (e) {
      return sendJson(res, 500, {
        ok: false,
        error: { code: "E_INTERNAL", message: String(e?.message || e) },
      });
    }
  }

  function listen(port = 0, host = "127.0.0.1") {
    return new Promise((resolve, reject) => {
      const server = http.createServer((req, res) => {
        handler(req, res);
      });
      server.once("error", reject);
      server.listen(port, host, () => {
        const addr = server.address();
        resolve({
          server,
          port: addr.port,
          host: addr.address,
          url: `http://${addr.address}:${addr.port}`,
          close: () =>
            new Promise((resClose, rejClose) =>
              server.close((err) => (err ? rejClose(err) : resClose()))
            ),
        });
      });
    });
  }

  return { handler, listen, store };
}

function sendJson(res, status, obj) {
  const body = JSON.stringify(obj);
  res.writeHead(status, {
    "Content-Type": "application/json; charset=utf-8",
    "Content-Length": Buffer.byteLength(body),
  });
  res.end(body);
}

function readJson(req) {
  return new Promise((resolve, reject) => {
    const chunks = [];
    req.on("data", (c) => chunks.push(c));
    req.on("end", () => {
      try {
        const raw = Buffer.concat(chunks).toString("utf8");
        resolve(raw ? JSON.parse(raw) : {});
      } catch (e) {
        reject(e);
      }
    });
    req.on("error", reject);
  });
}

export default { createApp };
