#!/usr/bin/env node
/**
 * Testing Hub marketplace CLI (local registry + optional remote/local server).
 *
 *   node marketplace.mjs list [--json]
 *   node marketplace.mjs verify --zip file.zip --meta meta.json
 *   node marketplace.mjs install --zip file.zip --meta meta.json [--data-root d]
 *   node marketplace.mjs uninstall --plugin id --version x.y.z
 *   node marketplace.mjs catalog [--url http://127.0.0.1:8787]
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
  if (rest[0] && !rest[0].startsWith("-")) out.cmd = rest.shift();
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
  node marketplace.mjs verify --zip <file> --meta <json>
  node marketplace.mjs install --zip <file> --meta <json> [--data-root d]
  node marketplace.mjs uninstall --plugin <id> --version <ver>
  node marketplace.mjs activate --plugin <id> --version <ver>
  node marketplace.mjs catalog [--url URL] [--channel stable] [--json]
  node marketplace.mjs pull --plugin <id> --version <ver> [--url URL]

Local loopback (HTTP):
  WIPARSE_MARKETPLACE_ALLOW_HTTP=1 node marketplace.mjs catalog --url http://127.0.0.1:8787

Env: WIPARSE_MARKETPLACE_URL, WIPARSE_MARKETPLACE_TOKEN, WIPARSE_MARKETPLACE_ALLOW_HTTP, WIPARSE_DATA_ROOT
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

function emit(opts, obj) {
  console.log(typeof obj === "string" ? obj : JSON.stringify(obj, null, 2));
  process.exit(0);
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
      emit(opts, { ok: true, install_dir: root, plugins: listInstalled(root) });
    }

    if (opts.cmd === "verify") {
      if (!opts.zip || !opts.meta) {
        throw new MarketplaceError("E_CONFIG", "need --zip and --meta");
      }
      const meta = JSON.parse(fs.readFileSync(opts.meta, "utf8"));
      const result = verifyArtifactIntegrity({
        meta,
        filePath: path.resolve(opts.zip),
        trust,
      });
      emit(opts, { ok: true, ...result, id: meta.id, version: meta.version });
    }

    if (opts.cmd === "install") {
      if (!opts.zip || !opts.meta) {
        throw new MarketplaceError("E_CONFIG", "need --zip and --meta");
      }
      const root = rootFromOpts(opts);
      const meta = JSON.parse(fs.readFileSync(opts.meta, "utf8"));
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
      emit(opts, uninstallVersion(rootFromOpts(opts), opts.plugin, opts.version));
    }

    if (opts.cmd === "activate") {
      if (!opts.plugin || !opts.version) {
        throw new MarketplaceError("E_CONFIG", "need --plugin and --version");
      }
      const entry = setActiveVersion(rootFromOpts(opts), opts.plugin, opts.version);
      emit(opts, { ok: true, id: opts.plugin, active: entry.active });
    }

    if (opts.cmd === "catalog") {
      const url = opts.url || process.env.WIPARSE_MARKETPLACE_URL || "";
      if (!url) throw new MarketplaceError("E_CONFIG", "need --url or WIPARSE_MARKETPLACE_URL");
      const client = createMarketplaceClient({
        baseUrl: url,
        token: opts.token || process.env.WIPARSE_MARKETPLACE_TOKEN,
        trust,
        allowHttp: allowHttpFromEnv(),
      });
      emit(opts, await client.catalog({ channel: opts.channel || undefined }));
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
      const result = installFromZip({
        marketplaceRoot: rootFromOpts(opts),
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
    console.error(
      `[marketplace] ${payload.error?.code || "E_INTERNAL"}: ${payload.error?.message || e}`
    );
    if (opts.json) console.log(JSON.stringify(payload, null, 2));
    process.exit(1);
  }
}

if (
  process.argv[1] &&
  path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)
) {
  main();
}
