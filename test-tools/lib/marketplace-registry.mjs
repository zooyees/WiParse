/**
 * Local marketplace registry: installed plugins index under install root.
 *
 * Layout:
 *   {root}/registry.json
 *   {root}/plugins/<id>/<version>/...
 *   {root}/cache/
 */

import fs from "node:fs";
import path from "node:path";
import {
  MarketplaceError,
  MARKETPLACE_ERROR,
  validatePackageMeta,
} from "./marketplace-trust.mjs";

const REGISTRY_VERSION = 1;

export function defaultRegistry() {
  return {
    version: REGISTRY_VERSION,
    updated_at: null,
    plugins: {},
  };
}

export function resolveMarketplaceRoot({ installDir, dataRoot } = {}) {
  if (installDir && String(installDir).trim()) {
    return path.resolve(String(installDir).trim());
  }
  const root = dataRoot && String(dataRoot).trim()
    ? path.resolve(String(dataRoot).trim())
    : process.cwd();
  return path.join(root, "marketplace");
}

export function pluginsInstallRoot(marketplaceRoot) {
  return path.join(marketplaceRoot, "plugins");
}

export function cacheRoot(marketplaceRoot) {
  return path.join(marketplaceRoot, "cache");
}

export function registryPath(marketplaceRoot) {
  return path.join(marketplaceRoot, "registry.json");
}

export function pluginVersionDir(marketplaceRoot, id, version) {
  assertSafeId(id);
  assertSafeVersion(version);
  return path.join(pluginsInstallRoot(marketplaceRoot), id, version);
}

export function assertSafeId(id) {
  if (!id || !/^[a-zA-Z0-9][a-zA-Z0-9._-]*$/.test(id)) {
    throw new MarketplaceError(MARKETPLACE_ERROR.LAYOUT, `invalid plugin id: ${id}`);
  }
}

export function assertSafeVersion(version) {
  const v = String(version || "").trim();
  if (!v || v.includes("..") || v.includes("/") || v.includes("\\") || v.includes("\0")) {
    throw new MarketplaceError(MARKETPLACE_ERROR.LAYOUT, `invalid version: ${version}`);
  }
  if (!/^[a-zA-Z0-9][a-zA-Z0-9._+-]*$/.test(v)) {
    throw new MarketplaceError(MARKETPLACE_ERROR.LAYOUT, `invalid version chars: ${version}`);
  }
}

export function ensureMarketplaceLayout(marketplaceRoot) {
  fs.mkdirSync(pluginsInstallRoot(marketplaceRoot), { recursive: true });
  fs.mkdirSync(cacheRoot(marketplaceRoot), { recursive: true });
  const rp = registryPath(marketplaceRoot);
  if (!fs.existsSync(rp)) {
    writeRegistry(marketplaceRoot, defaultRegistry());
  }
}

export function readRegistry(marketplaceRoot) {
  const rp = registryPath(marketplaceRoot);
  if (!fs.existsSync(rp)) return defaultRegistry();
  try {
    const raw = JSON.parse(fs.readFileSync(rp, "utf8"));
    if (!raw || typeof raw !== "object" || Array.isArray(raw)) {
      return defaultRegistry();
    }
    if (!raw.plugins || typeof raw.plugins !== "object") {
      raw.plugins = {};
    }
    return raw;
  } catch (e) {
    throw new MarketplaceError(
      MARKETPLACE_ERROR.LAYOUT,
      `corrupt registry.json: ${e.message}`
    );
  }
}

export function writeRegistry(marketplaceRoot, registry) {
  fs.mkdirSync(marketplaceRoot, { recursive: true });
  const next = {
    ...registry,
    version: REGISTRY_VERSION,
    updated_at: new Date().toISOString(),
  };
  const rp = registryPath(marketplaceRoot);
  const tmp = `${rp}.${process.pid}.tmp`;
  fs.writeFileSync(tmp, JSON.stringify(next, null, 2), "utf8");
  fs.renameSync(tmp, rp);
  return next;
}

/**
 * Record or update an installed version; optionally set active.
 */
export function recordInstall(marketplaceRoot, meta, { activate = true, source = "local" } = {}) {
  const check = validatePackageMeta(meta);
  if (!check.ok) {
    throw new MarketplaceError(MARKETPLACE_ERROR.LAYOUT, check.errors.join("; "));
  }
  assertSafeId(meta.id);
  assertSafeVersion(meta.version);
  const dir = pluginVersionDir(marketplaceRoot, meta.id, meta.version);
  if (!fs.existsSync(path.join(dir, "plugin.json"))) {
    throw new MarketplaceError(
      MARKETPLACE_ERROR.LAYOUT,
      `missing plugin.json under ${dir}`
    );
  }
  const reg = readRegistry(marketplaceRoot);
  const entry = reg.plugins[meta.id] || {
    id: meta.id,
    active: null,
    versions: {},
  };
  entry.versions[meta.version] = {
    version: meta.version,
    installed_at: new Date().toISOString(),
    dir,
    sha256: String(meta.sha256).toLowerCase(),
    size: meta.size,
    publisher: meta.publisher || null,
    channel: meta.channel || "stable",
    engines: meta.engines || undefined,
    sandbox: meta.sandbox || undefined,
    source,
    signature: meta.signature || null,
  };
  if (activate) entry.active = meta.version;
  reg.plugins[meta.id] = entry;
  writeRegistry(marketplaceRoot, reg);
  return entry;
}

export function setActiveVersion(marketplaceRoot, id, version) {
  assertSafeId(id);
  assertSafeVersion(version);
  const reg = readRegistry(marketplaceRoot);
  const entry = reg.plugins[id];
  if (!entry || !entry.versions?.[version]) {
    throw new MarketplaceError(
      MARKETPLACE_ERROR.NOT_FOUND,
      `installed version not found: ${id}@${version}`
    );
  }
  entry.active = version;
  writeRegistry(marketplaceRoot, reg);
  return entry;
}

export function uninstallVersion(marketplaceRoot, id, version) {
  assertSafeId(id);
  assertSafeVersion(version);
  const reg = readRegistry(marketplaceRoot);
  const entry = reg.plugins[id];
  if (!entry || !entry.versions?.[version]) {
    throw new MarketplaceError(
      MARKETPLACE_ERROR.NOT_FOUND,
      `installed version not found: ${id}@${version}`
    );
  }
  const dir = entry.versions[version].dir || pluginVersionDir(marketplaceRoot, id, version);
  fs.rmSync(dir, { recursive: true, force: true });
  delete entry.versions[version];
  if (entry.active === version) {
    const remaining = Object.keys(entry.versions).sort();
    entry.active = remaining.length ? remaining[remaining.length - 1] : null;
  }
  if (!Object.keys(entry.versions).length) {
    delete reg.plugins[id];
    const parent = path.join(pluginsInstallRoot(marketplaceRoot), id);
    fs.rmSync(parent, { recursive: true, force: true });
  } else {
    reg.plugins[id] = entry;
  }
  writeRegistry(marketplaceRoot, reg);
  return { ok: true, id, version };
}

export function listInstalled(marketplaceRoot) {
  const reg = readRegistry(marketplaceRoot);
  const out = [];
  for (const [id, entry] of Object.entries(reg.plugins || {})) {
    const active = entry.active;
    const ver = active ? entry.versions?.[active] : null;
    out.push({
      id,
      active,
      version: active,
      dir: ver?.dir || null,
      publisher: ver?.publisher || null,
      channel: ver?.channel || null,
      versions: Object.keys(entry.versions || {}).sort(),
      source: ver?.source || null,
    });
  }
  out.sort((a, b) => a.id.localeCompare(b.id));
  return out;
}

/**
 * Resolve active installed plugin dirs for discovery merge.
 * @returns {{ id: string, version: string, dir: string }[]}
 */
export function resolveActivePluginDirs(marketplaceRoot) {
  if (!marketplaceRoot || !fs.existsSync(registryPath(marketplaceRoot))) {
    return [];
  }
  const reg = readRegistry(marketplaceRoot);
  const out = [];
  for (const [id, entry] of Object.entries(reg.plugins || {})) {
    if (!entry.active) continue;
    const ver = entry.versions?.[entry.active];
    if (!ver?.dir) continue;
    if (!fs.existsSync(path.join(ver.dir, "plugin.json"))) continue;
    out.push({ id, version: entry.active, dir: ver.dir, source: "marketplace" });
  }
  return out;
}

export default {
  defaultRegistry,
  resolveMarketplaceRoot,
  pluginsInstallRoot,
  cacheRoot,
  registryPath,
  pluginVersionDir,
  assertSafeId,
  assertSafeVersion,
  ensureMarketplaceLayout,
  readRegistry,
  writeRegistry,
  recordInstall,
  setActiveVersion,
  uninstallVersion,
  listInstalled,
  resolveActivePluginDirs,
};
