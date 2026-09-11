#!/usr/bin/env node
/**
 * Testing Hub marketplace CLI (local registry + optional cloud).
 *
 *   node marketplace.mjs list [--json]
 *   node marketplace.mjs verify --zip file.zip --meta meta.json [--json]
 *   node marketplace.mjs install --zip file.zip --meta meta.json [--install-dir d]
 *   node marketplace.mjs uninstall --plugin id --version x.y.z
 *   node marketplace.mjs catalog [--url https://...] [--channel stable]
 *   node marketplace.mjs pull --plugin id --version x.y.z [--url ...]
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { resolveDataRoot } from "./lib/plugin-contract.mjs";
import {
  MarketplaceError,
  verifyArtifactIntegrity,
  normalizeTrustPolicy,
  allowHttpFromEnv,
} from "./lib/marketplace-trust.mjs";
import {
  resolveMarketplaceRoot,
  ensureMarketplaceLayout,
  listInstalled,
  uninstallVersion,
  setActiveVersion,
} from "./lib/marketplace-registry.mjs";
import { installFromZip, installFromZipFile } from "./lib/marketplace-install.mjs";
import { createMarketplaceClient } from "./lib/marketplace-client.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

function parseArgs(argv) {
  const out = {
    cmd: null,
    json: false,
    zip: null,
    meta: null,
    plugin: null,
    version: null,
    url: null,
    token: null,
    installDir: null,
    dataRoot: null,
    channel: null,
    activate: true,
    trustRequireSig: false,
    help: false,
  };
  const rest = [...argv];
  if (rest[0] && !rest[0].startsWith("-")) {
    out.cmd = rest.shift();
  }
  for (let i = 0; i < rest.length; i++) {
    const a = rest[i];
    if (a === "--json") out.json = true;
    else if (a === "--help" || a === "-h") out.help = true;
    else if (a === "--no-activate") out.activate = false;
    else if (a === "--require-signature") out.trustRequireSig = true;
    else if (a === "--zip" && rest[i + 1]) out.zip = rest[++i];
    else if (a === "--meta" && rest[i + 1]) out.meta = rest[++i];
    else if ((a === "--plugin" || a === "--id") && rest[i + 1]) out.plugin = rest[++i];
    else if (a === "--version" && rest[i + 1]) out.version = rest[++i];
    else if ((a === "--url" || a === "--base-url") && rest[i + 1]) out.url = rest[++i];
    else if (a === "--token" && rest[i + 1]) out.token = rest[++i];
    else if ((a === "--install-dir" || a === "--install_dir") && rest[i + 1]) {
      out.installDir = rest[++i];
    } else if ((a === "--data-root" || a === "--data_root") && rest[i + 1]) {
      out.dataRoot = rest[++i];
    } else if (a === "--channel" && rest[i + 1]) out.channel = rest[++i];
    else {
      console.error(`unknown arg: ${a}`);
      process.exit(2);
    }
  }
  return out;
}

function printHelp() {
  console.log(`Usage:
  node marketplace.mjs list [--install-dir d] [--json]
  node marketplace.mjs verify --zip <file> --meta <json> [--require-signature]
  node marketplace.mjs install --zip <file> --meta <json> [--install-dir d]
  node marketplace.mjs uninstall --plugin <id> --version <ver> [--install-dir d]
  node marketplace.mjs activate --plugin <id> --version <ver>
  node marketplace.mjs catalog [--url URL] [--channel stable] [--json]
  node marketplace.mjs pull --plugin <id> --version <ver> [--url URL] [--install-dir d]

Env: WIPARSE_MARKETPLACE_URL, WIPARSE_MARKETPLACE_TOKEN, WIPARSE_MARKETPLACE_ALLOW_HTTP,
     WIPARSE_DATA_ROOT
`);
}

function trustFromOpts(opts) {
  return normalizeTrustPolicy({
    require_signature: opts.trustRequireSig,
    public_keys: (process.env.WIPARSE_MARKETPLACE_PUBKEYS || "")
      .split(/[,;]/)
      .map((s) => s.trim())
      .filter(Boolean),
    allowed_publishers: (process.env.WIPARSE_MARKETPLACE_ALLOW_PUBLISHERS || "")
      .split(/[,;]/)
      .map((s) => s.trim())
      .filter(Boolean),
    denied_publishers: (process.env.WIPARSE_MARKETPLACE_DENY_PUBLISHERS || "")
      .split(/[,;]/)
      .map((s) => s.trim())
      .filter(Boolean),
  });
}

function loadMeta(p) {
  return JSON.parse(fs.readFileSync(p, "utf8"));
}

function emit(opts, obj, exitCode = 0) {
  if (opts.json) console.log(JSON.stringify(obj, null, 2));
  else if (obj?.ok === false) console.error(JSON.stringify(obj));
  else if (typeof obj === "string") console.log(obj);
  else console.log(JSON.stringify(obj, null, 2));
  process.exit(exitCode);
}

function rootFromOpts(opts) {
  const dataRoot = resolveDataRoot(opts.dataRoot);
  return resolveMarketplaceRoot({
    installDir: opts.installDir || process.env.WIPARSE_MARKETPLACE_INSTALL_DIR,
    dataRoot,
  });
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  if (opts.help || !opts.cmd) {
    printHelp();
    process.exit(opts.help ? 0 : 2);
  }

  try {
    const trust = trustFromOpts(opts);

    if (opts.cmd === "list") {
      const root = rootFromOpts(opts);
      ensureMarketplaceLayout(root);
      const items = listInstalled(root);
      emit(opts, { ok: true, install_dir: root, plugins: items });
    }

    if (opts.cmd === "verify") {
      if (!opts.zip || !opts.meta) throw new MarketplaceError("E_CONFIG", "need --zip and --meta");
      const meta = loadMeta(opts.meta);
      const result = verifyArtifactIntegrity({
        meta,
        filePath: path.resolve(opts.zip),
        trust,
      });
      emit(opts, { ok: true, ...result, id: meta.id, version: meta.version });
    }

    if (opts.cmd === "install") {
      if (!opts.zip || !opts.meta) throw new MarketplaceError("E_CONFIG", "need --zip and --meta");
      const root = rootFromOpts(opts);
      const meta = loadMeta(opts.meta);
      const result = installFromZipFile({
        marketplaceRoot: root,
        meta,
        zipPath: path.resolve(opts.zip),
        trust,
        activate: opts.activate,
        source: "local-zip",
      });
      emit(opts, result);
    }

    if (opts.cmd === "uninstall") {
      if (!opts.plugin || !opts.version) {
        throw new MarketplaceError("E_CONFIG", "need --plugin and --version");
      }
      const root = rootFromOpts(opts);
      emit(opts, uninstallVersion(root, opts.plugin, opts.version));
    }

    if (opts.cmd === "activate") {
      if (!opts.plugin || !opts.version) {
        throw new MarketplaceError("E_CONFIG", "need --plugin and --version");
      }
      const root = rootFromOpts(opts);
      const entry = setActiveVersion(root, opts.plugin, opts.version);
      emit(opts, { ok: true, id: opts.plugin, active: entry.active });
    }

    if (opts.cmd === "catalog") {
      const url =
        opts.url ||
        process.env.WIPARSE_MARKETPLACE_URL ||
        "";
      if (!url) throw new MarketplaceError("E_CONFIG", "need --url or WIPARSE_MARKETPLACE_URL");
      const client = createMarketplaceClient({
        baseUrl: url,
        token: opts.token || process.env.WIPARSE_MARKETPLACE_TOKEN,
        trust,
        allowHttp: allowHttpFromEnv(),
      });
      const data = await client.catalog({
        channel: opts.channel || undefined,
      });
      emit(opts, data);
    }

    if (opts.cmd === "pull") {
      if (!opts.plugin || !opts.version) {
        throw new MarketplaceError("E_CONFIG", "need --plugin and --version");
      }
      const url = opts.url || process.env.WIPARSE_MARKETPLACE_URL || "";
      if (!url) throw new MarketplaceError("E_CONFIG", "need --url or WIPARSE_MARKETPLACE_URL");
      const client = createMarketplaceClient({
        baseUrl: url,
        token: opts.token || process.env.WIPARSE_MARKETPLACE_TOKEN,
        trust,
        allowHttp: allowHttpFromEnv(),
      });
      const metaResp = await client.getVersion(opts.plugin, opts.version);
      const meta = metaResp.version || metaResp;
      const zipBytes = await client.download(opts.plugin, opts.version);
      const root = rootFromOpts(opts);
      const result = installFromZip({
        marketplaceRoot: root,
        meta,
        zipBytes,
        trust,
        activate: opts.activate,
        source: url,
      });
      emit(opts, result);
    }

    throw new MarketplaceError("E_CONFIG", `unknown command: ${opts.cmd}`);
  } catch (e) {
    const payload =
      e instanceof MarketplaceError
        ? e.toJSON()
        : { ok: false, error: { code: "E_INTERNAL", message: String(e?.message || e) } };
    if (opts.json) console.log(JSON.stringify(payload, null, 2));
    else console.error(`[marketplace] ${payload.error.code}: ${payload.error.message}`);
    process.exit(1);
  }
}

const isDirect =
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url);

if (isDirect) {
  main();
}
