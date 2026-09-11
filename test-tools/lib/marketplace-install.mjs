/**
 * Install marketplace plugin packages (zip) into the local registry tree.
 */

import fs from "node:fs";
import path from "node:path";
import zlib from "node:zlib";
import {
  MarketplaceError,
  MARKETPLACE_ERROR,
  verifyArtifactIntegrity,
  validatePackageMeta,
  sha256Hex,
} from "./marketplace-trust.mjs";
import {
  ensureMarketplaceLayout,
  cacheRoot,
  pluginVersionDir,
  recordInstall,
  assertSafeId,
  assertSafeVersion,
} from "./marketplace-registry.mjs";

/**
 * Minimal zip reader (store + deflate) without npm deps.
 * Supports standard local file headers only.
 */
export function unzipToDirectory(zipBytes, destDir, { maxFiles = 5000, maxTotalBytes = 200 * 1024 * 1024 } = {}) {
  const buf = Buffer.isBuffer(zipBytes) ? zipBytes : Buffer.from(zipBytes);
  let offset = 0;
  let files = 0;
  let total = 0;
  const written = [];

  while (offset + 30 <= buf.length) {
    const sig = buf.readUInt32LE(offset);
    if (sig !== 0x04034b50) break; // local file header
    const method = buf.readUInt16LE(offset + 8);
    const compSize = buf.readUInt32LE(offset + 18);
    const uncompSize = buf.readUInt32LE(offset + 22);
    const nameLen = buf.readUInt16LE(offset + 26);
    const extraLen = buf.readUInt16LE(offset + 28);
    const nameStart = offset + 30;
    const nameEnd = nameStart + nameLen;
    const dataStart = nameEnd + extraLen;
    const dataEnd = dataStart + compSize;
    if (dataEnd > buf.length) {
      throw new MarketplaceError(MARKETPLACE_ERROR.LAYOUT, "truncated zip entry");
    }
    const name = buf.subarray(nameStart, nameEnd).toString("utf8");
    offset = dataEnd;
    if (!name || name.endsWith("/")) continue;

    assertSafeZipEntry(name);
    files += 1;
    if (files > maxFiles) {
      throw new MarketplaceError(MARKETPLACE_ERROR.LAYOUT, "zip has too many files");
    }

    let data;
    if (method === 0) {
      data = buf.subarray(dataStart, dataEnd);
    } else if (method === 8) {
      data = zlib.inflateRawSync(buf.subarray(dataStart, dataEnd));
    } else {
      throw new MarketplaceError(MARKETPLACE_ERROR.LAYOUT, `unsupported zip method ${method}`);
    }
    if (uncompSize && data.length !== uncompSize) {
      // Allow mismatch when zip64 / data descriptor omitted uncomp; still bound size
    }
    total += data.length;
    if (total > maxTotalBytes) {
      throw new MarketplaceError(MARKETPLACE_ERROR.LAYOUT, "zip uncompressed size too large");
    }

    const outPath = path.join(destDir, name);
    const resolved = path.resolve(outPath);
    if (!resolved.startsWith(path.resolve(destDir) + path.sep) && resolved !== path.resolve(destDir)) {
      throw new MarketplaceError(MARKETPLACE_ERROR.LAYOUT, `zip path escape: ${name}`);
    }
    fs.mkdirSync(path.dirname(resolved), { recursive: true });
    fs.writeFileSync(resolved, data);
    written.push(name);
  }

  if (!written.length) {
    throw new MarketplaceError(MARKETPLACE_ERROR.LAYOUT, "zip contained no files");
  }
  return written;
}

function assertSafeZipEntry(name) {
  const n = name.replace(/\\/g, "/");
  if (n.startsWith("/") || /^[A-Za-z]:/.test(n)) {
    throw new MarketplaceError(MARKETPLACE_ERROR.LAYOUT, `absolute zip path: ${name}`);
  }
  const parts = n.split("/");
  if (parts.some((p) => p === ".." || p === "")) {
    // allow empty only from trailing slash (already skipped); reject ..
    if (parts.includes("..")) {
      throw new MarketplaceError(MARKETPLACE_ERROR.LAYOUT, `zip path escape: ${name}`);
    }
  }
}

/**
 * If zip has a single top-level directory, return its path; else dest itself.
 */
export function resolvePluginRootAfterUnzip(destDir) {
  const entries = fs.readdirSync(destDir).filter((n) => n !== "__MACOSX");
  if (entries.length === 1) {
    const only = path.join(destDir, entries[0]);
    if (fs.statSync(only).isDirectory() && fs.existsSync(path.join(only, "plugin.json"))) {
      return only;
    }
  }
  if (fs.existsSync(path.join(destDir, "plugin.json"))) return destDir;
  // search one level
  for (const name of entries) {
    const p = path.join(destDir, name);
    if (fs.statSync(p).isDirectory() && fs.existsSync(path.join(p, "plugin.json"))) {
      return p;
    }
  }
  throw new MarketplaceError(MARKETPLACE_ERROR.LAYOUT, "plugin.json not found in package");
}

function copyDirRecursive(src, dest) {
  fs.mkdirSync(dest, { recursive: true });
  for (const ent of fs.readdirSync(src, { withFileTypes: true })) {
    if (ent.name === "." || ent.name === "..") continue;
    const from = path.join(src, ent.name);
    const to = path.join(dest, ent.name);
    if (ent.isDirectory()) copyDirRecursive(from, to);
    else if (ent.isSymbolicLink()) {
      throw new MarketplaceError(MARKETPLACE_ERROR.LAYOUT, `symlinks not allowed: ${ent.name}`);
    } else if (ent.isFile()) {
      fs.copyFileSync(from, to);
    }
  }
}

/**
 * Build a simple zip (store method) from a directory — used by tests / pack helper.
 */
export function zipDirectory(dir) {
  const files = [];
  function walk(rel) {
    const abs = path.join(dir, rel);
    for (const ent of fs.readdirSync(abs, { withFileTypes: true })) {
      const r = rel ? `${rel}/${ent.name}` : ent.name;
      if (ent.isDirectory()) walk(r);
      else if (ent.isFile()) files.push(r.replace(/\\/g, "/"));
    }
  }
  walk("");
  const parts = [];
  const central = [];
  let offset = 0;
  for (const name of files) {
    const data = fs.readFileSync(path.join(dir, name));
    const nameBuf = Buffer.from(name, "utf8");
    const local = Buffer.alloc(30);
    local.writeUInt32LE(0x04034b50, 0);
    local.writeUInt16LE(20, 4); // version needed
    local.writeUInt16LE(0, 6);
    local.writeUInt16LE(0, 8); // store
    local.writeUInt16LE(0, 10);
    local.writeUInt16LE(0, 12);
    local.writeUInt32LE(0, 14); // crc skipped for store tooling simplicity
    local.writeUInt32LE(data.length, 18);
    local.writeUInt32LE(data.length, 22);
    local.writeUInt16LE(nameBuf.length, 26);
    local.writeUInt16LE(0, 28);
    parts.push(local, nameBuf, data);

    const cen = Buffer.alloc(46);
    cen.writeUInt32LE(0x02014b50, 0);
    cen.writeUInt16LE(20, 4);
    cen.writeUInt16LE(20, 6);
    cen.writeUInt16LE(0, 8);
    cen.writeUInt16LE(0, 10);
    cen.writeUInt16LE(0, 12);
    cen.writeUInt16LE(0, 14);
    cen.writeUInt32LE(0, 16);
    cen.writeUInt32LE(data.length, 20);
    cen.writeUInt32LE(data.length, 24);
    cen.writeUInt16LE(nameBuf.length, 28);
    cen.writeUInt16LE(0, 30);
    cen.writeUInt16LE(0, 32);
    cen.writeUInt16LE(0, 34);
    cen.writeUInt16LE(0, 36);
    cen.writeUInt32LE(0, 38);
    cen.writeUInt32LE(offset, 42);
    central.push(cen, nameBuf);
    offset += 30 + nameBuf.length + data.length;
  }
  const centralBuf = Buffer.concat(central);
  const end = Buffer.alloc(22);
  end.writeUInt32LE(0x06054b50, 0);
  end.writeUInt16LE(0, 4);
  end.writeUInt16LE(0, 6);
  end.writeUInt16LE(files.length, 8);
  end.writeUInt16LE(files.length, 10);
  end.writeUInt32LE(centralBuf.length, 12);
  end.writeUInt32LE(offset, 16);
  end.writeUInt16LE(0, 20);
  return Buffer.concat([...parts, centralBuf, end]);
}

/**
 * Install from zip bytes + metadata. Verifies integrity before promoting.
 */
export function installFromZip({
  marketplaceRoot,
  meta,
  zipBytes,
  trust = null,
  activate = true,
  source = "local",
} = {}) {
  if (!marketplaceRoot) {
    throw new MarketplaceError(MARKETPLACE_ERROR.CONFIG, "marketplaceRoot required");
  }
  const check = validatePackageMeta(meta);
  if (!check.ok) {
    throw new MarketplaceError(MARKETPLACE_ERROR.LAYOUT, check.errors.join("; "));
  }
  assertSafeId(meta.id);
  assertSafeVersion(meta.version);

  const buf = Buffer.isBuffer(zipBytes) ? zipBytes : Buffer.from(zipBytes);
  verifyArtifactIntegrity({ meta, bytes: buf, trust });

  ensureMarketplaceLayout(marketplaceRoot);
  const cache = path.join(
    cacheRoot(marketplaceRoot),
    `${meta.id}-${meta.version}-${process.pid}`
  );
  fs.rmSync(cache, { recursive: true, force: true });
  fs.mkdirSync(cache, { recursive: true });

  try {
    unzipToDirectory(buf, cache);
    const pluginRoot = resolvePluginRootAfterUnzip(cache);
    const manifest = JSON.parse(fs.readFileSync(path.join(pluginRoot, "plugin.json"), "utf8"));
    if (String(manifest.id) !== String(meta.id)) {
      throw new MarketplaceError(
        MARKETPLACE_ERROR.LAYOUT,
        `package id mismatch: meta=${meta.id} plugin.json=${manifest.id}`
      );
    }
    if (manifest.version && String(manifest.version) !== String(meta.version)) {
      throw new MarketplaceError(
        MARKETPLACE_ERROR.LAYOUT,
        `package version mismatch: meta=${meta.version} plugin.json=${manifest.version}`
      );
    }

    const dest = pluginVersionDir(marketplaceRoot, meta.id, meta.version);
    fs.rmSync(dest, { recursive: true, force: true });
    fs.mkdirSync(path.dirname(dest), { recursive: true });
    // Move/copy atomically-ish: copy then remove cache
    copyDirRecursive(pluginRoot, dest);
    const entry = recordInstall(marketplaceRoot, meta, { activate, source });
    return {
      ok: true,
      id: meta.id,
      version: meta.version,
      dir: dest,
      active: entry.active,
      sha256: String(meta.sha256).toLowerCase(),
    };
  } finally {
    fs.rmSync(cache, { recursive: true, force: true });
  }
}

export function installFromZipFile(opts) {
  const zipBytes = fs.readFileSync(opts.zipPath);
  return installFromZip({ ...opts, zipBytes });
}

/** Create meta from zip bytes + plugin fields. */
export function buildMetaFromZip(zipBytes, fields = {}) {
  const buf = Buffer.isBuffer(zipBytes) ? zipBytes : Buffer.from(zipBytes);
  return {
    id: fields.id,
    version: fields.version,
    name: fields.name,
    name_zh: fields.name_zh,
    type: fields.type,
    description: fields.description,
    publisher: fields.publisher || "local",
    channel: fields.channel || "stable",
    engines: fields.engines,
    sandbox: fields.sandbox,
    sha256: sha256Hex(buf),
    size: buf.length,
    signature: fields.signature ?? null,
    published_at: fields.published_at || new Date().toISOString(),
  };
}

export default {
  unzipToDirectory,
  resolvePluginRootAfterUnzip,
  zipDirectory,
  installFromZip,
  installFromZipFile,
  buildMetaFromZip,
};
