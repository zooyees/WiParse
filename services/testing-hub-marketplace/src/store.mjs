/**
 * Filesystem-backed marketplace store: index.json + artifacts/<id>/<version>.zip
 */

import fs from "node:fs";
import path from "node:path";
import crypto from "node:crypto";

export function createStore(dataDir) {
  const root = path.resolve(dataDir);
  const artifacts = path.join(root, "artifacts");
  const indexPath = path.join(root, "index.json");

  function ensure() {
    fs.mkdirSync(artifacts, { recursive: true });
    if (!fs.existsSync(indexPath)) {
      writeIndex({ version: 1, updated_at: null, plugins: {} });
    }
  }

  function readIndex() {
    ensure();
    return JSON.parse(fs.readFileSync(indexPath, "utf8"));
  }

  function writeIndex(idx) {
    fs.mkdirSync(root, { recursive: true });
    const next = { ...idx, version: 1, updated_at: new Date().toISOString() };
    const tmp = `${indexPath}.${process.pid}.tmp`;
    fs.writeFileSync(tmp, JSON.stringify(next, null, 2), "utf8");
    fs.renameSync(tmp, indexPath);
    return next;
  }

  function artifactPath(id, version) {
    return path.join(artifacts, id, `${version}.zip`);
  }

  function listCatalog({ channel, type, q } = {}) {
    const idx = readIndex();
    const out = [];
    for (const [id, plug] of Object.entries(idx.plugins || {})) {
      const versions = Object.keys(plug.versions || {}).sort(cmpSemverDesc);
      if (!versions.length) continue;
      const latest = plug.versions[versions[0]];
      if (channel && latest.channel && latest.channel !== channel) continue;
      if (type && latest.type && latest.type !== type) continue;
      if (q) {
        const hay = `${id} ${latest.name || ""} ${latest.name_zh || ""} ${latest.description || ""}`.toLowerCase();
        if (!hay.includes(String(q).toLowerCase())) continue;
      }
      out.push({
        id,
        name: latest.name || id,
        name_zh: latest.name_zh || undefined,
        type: latest.type || "custom",
        publisher: latest.publisher || undefined,
        channel: latest.channel || "stable",
        latest_version: versions[0],
        description: latest.description || "",
        versions,
      });
    }
    out.sort((a, b) => a.id.localeCompare(b.id));
    return out;
  }

  function getPlugin(id) {
    const idx = readIndex();
    const plug = idx.plugins?.[id];
    if (!plug) return null;
    const versions = Object.keys(plug.versions || {}).sort(cmpSemverDesc);
    return {
      id,
      versions: versions.map((v) => plug.versions[v]),
      latest_version: versions[0] || null,
    };
  }

  function getVersion(id, version) {
    const idx = readIndex();
    return idx.plugins?.[id]?.versions?.[version] || null;
  }

  function putVersion(meta, zipBytes) {
    const id = meta.id;
    const version = meta.version;
    assertSafe(id, version);
    const buf = Buffer.isBuffer(zipBytes) ? zipBytes : Buffer.from(zipBytes);
    const sha = crypto.createHash("sha256").update(buf).digest("hex");
    if (String(meta.sha256).toLowerCase() !== sha) {
      const err = new Error(`sha256 mismatch: meta=${meta.sha256} actual=${sha}`);
      err.code = "E_HASH";
      throw err;
    }
    if (Number(meta.size) !== buf.length) {
      const err = new Error(`size mismatch: meta=${meta.size} actual=${buf.length}`);
      err.code = "E_HASH";
      throw err;
    }

    const dest = artifactPath(id, version);
    fs.mkdirSync(path.dirname(dest), { recursive: true });
    const tmp = `${dest}.${process.pid}.tmp`;
    fs.writeFileSync(tmp, buf);
    fs.renameSync(tmp, dest);

    const idx = readIndex();
    if (!idx.plugins[id]) idx.plugins[id] = { id, versions: {} };
    const stored = {
      ...meta,
      sha256: sha,
      size: buf.length,
      download_path: `/v1/plugins/${encodeURIComponent(id)}/versions/${encodeURIComponent(version)}/download`,
    };
    idx.plugins[id].versions[version] = stored;
    writeIndex(idx);
    return stored;
  }

  function deleteVersion(id, version) {
    assertSafe(id, version);
    const idx = readIndex();
    if (!idx.plugins?.[id]?.versions?.[version]) return false;
    delete idx.plugins[id].versions[version];
    if (!Object.keys(idx.plugins[id].versions).length) {
      delete idx.plugins[id];
      fs.rmSync(path.join(artifacts, id), { recursive: true, force: true });
    } else {
      fs.rmSync(artifactPath(id, version), { force: true });
    }
    writeIndex(idx);
    return true;
  }

  function readArtifact(id, version) {
    const p = artifactPath(id, version);
    if (!fs.existsSync(p)) return null;
    return fs.readFileSync(p);
  }

  return {
    root,
    ensure,
    readIndex,
    listCatalog,
    getPlugin,
    getVersion,
    putVersion,
    deleteVersion,
    readArtifact,
    artifactPath,
  };
}

function assertSafe(id, version) {
  if (!/^[a-zA-Z0-9][a-zA-Z0-9._-]*$/.test(id)) {
    const err = new Error(`invalid id: ${id}`);
    err.code = "E_LAYOUT";
    throw err;
  }
  if (
    !version ||
    String(version).includes("..") ||
    /[\\/]/.test(version) ||
    !/^[a-zA-Z0-9][a-zA-Z0-9._+-]*$/.test(version)
  ) {
    const err = new Error(`invalid version: ${version}`);
    err.code = "E_LAYOUT";
    throw err;
  }
}

function parseSemver(v) {
  const m = String(v || "0")
    .replace(/^v/i, "")
    .match(/^(\d+)(?:\.(\d+))?(?:\.(\d+))?/);
  if (!m) return [0, 0, 0];
  return [parseInt(m[1], 10) || 0, parseInt(m[2], 10) || 0, parseInt(m[3], 10) || 0];
}

function cmpSemverDesc(a, b) {
  const pa = parseSemver(a);
  const pb = parseSemver(b);
  for (let i = 0; i < 3; i++) {
    if (pa[i] > pb[i]) return -1;
    if (pa[i] < pb[i]) return 1;
  }
  return String(b).localeCompare(String(a));
}

export default { createStore };
