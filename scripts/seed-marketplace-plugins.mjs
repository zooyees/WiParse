#!/usr/bin/env node
/**
 * Create sample Testing Hub marketplace plugins and publish them.
 *
 * Offline (write into server data dir):
 *   node scripts/seed-marketplace-plugins.mjs
 *   CLEAN=1 node scripts/seed-marketplace-plugins.mjs --data services/testing-hub-marketplace/data
 *
 * Live (HTTP publish to a running server):
 *   node scripts/seed-marketplace-plugins.mjs --url http://127.0.0.1:8787 --token dev-token
 */

import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createStore } from "../services/testing-hub-marketplace/src/store.mjs";
import {
  zipDirectory,
  buildMetaFromZip,
} from "../test-tools/lib/marketplace-install.mjs";
import { createMarketplaceClient } from "../test-tools/lib/marketplace-client.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "..");
const FIXTURES = path.join(ROOT, "services/testing-hub-marketplace/fixtures");
const DEFAULT_DATA = path.join(ROOT, "services/testing-hub-marketplace/data");

const PLUGINS = [
  {
    id: "demo-marketplace-plugin",
    version: "1.0.0",
    name: "Demo Marketplace Plugin",
    name_zh: "市场演示插件",
    type: "smoke",
    description: "Minimal smoke plugin for marketplace browse/install demos.",
    publisher: "wiparse",
    channel: "stable",
    entry: `export default async function run(ctx) {
  return { ok: true, plugin: "demo-marketplace-plugin", lifecycle: ctx.lifecycle };
}
export async function preflight() {
  return { ok: true, lifecycle: "preflight", checks: [{ id: "smoke", ok: true }] };
}
`,
  },
  {
    id: "market-echo",
    version: "1.0.0",
    name: "Market Echo",
    name_zh: "市场回显",
    type: "custom",
    description: "Echoes params/context to verify install + run wiring.",
    publisher: "wiparse",
    channel: "stable",
    entry: `export default async function run(ctx) {
  return {
    ok: true,
    plugin: "market-echo",
    echo: { params: ctx.params || {}, lifecycle: ctx.lifecycle },
  };
}
export async function preflight() {
  return { ok: true, lifecycle: "preflight", checks: [{ id: "echo-ready", ok: true }] };
}
`,
  },
  {
    id: "market-counter",
    version: "1.1.0",
    name: "Market Counter",
    name_zh: "市场计数器",
    type: "smoke",
    description: "Counts 1..N and returns a summary (good after-install run).",
    publisher: "wiparse",
    channel: "stable",
    entry: `export default async function run(ctx) {
  const n = Math.max(1, Number(ctx.params?.count ?? 3) || 3);
  const steps = [];
  for (let i = 1; i <= n; i++) steps.push(i);
  return { ok: true, plugin: "market-counter", count: n, steps };
}
export async function preflight() {
  return {
    ok: true,
    lifecycle: "preflight",
    checks: [
      { id: "node", ok: true },
      { id: "counter", ok: true, detail: "ready" },
    ],
  };
}
`,
  },
  {
    id: "market-preflight-lab",
    version: "0.9.0",
    name: "Market Preflight Lab",
    name_zh: "市场预检实验",
    type: "smoke",
    description: "Rich preflight checks for Testing Hub marketplace UI demos.",
    publisher: "wiparse",
    channel: "stable",
    entry: `export default async function run() {
  return { ok: true, plugin: "market-preflight-lab", note: "run after preflight" };
}
export async function preflight(ctx) {
  return {
    ok: true,
    lifecycle: "preflight",
    checks: [
      { id: "manifest", ok: true },
      { id: "sandbox", ok: true, detail: "gui.api+cli" },
      { id: "data-root", ok: Boolean(ctx?.dataRoot || ctx?.data_root || true) },
    ],
  };
}
`,
  },
];

function parseArgs(argv) {
  const out = { data: null, url: null, token: null, clean: false };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--data" && argv[i + 1]) out.data = argv[++i];
    else if ((a === "--url" || a === "--base-url") && argv[i + 1]) out.url = argv[++i];
    else if (a === "--token" && argv[i + 1]) out.token = argv[++i];
    else if (a === "--clean") out.clean = true;
  }
  if (process.env.CLEAN === "1") out.clean = true;
  return out;
}

function writeFixture(spec) {
  const dir = path.join(FIXTURES, spec.id);
  fs.mkdirSync(dir, { recursive: true });
  const pluginJson = {
    id: spec.id,
    name: spec.name,
    name_zh: spec.name_zh,
    type: spec.type,
    version: spec.version,
    entry: "index.mjs",
    capabilities: ["preflight", "run"],
    description: spec.description,
    publisher: spec.publisher,
    marketplace: { channel: spec.channel },
    sandbox: { permissions: ["gui.api", "cli"] },
    engines: { node: ">=18" },
  };
  fs.writeFileSync(path.join(dir, "plugin.json"), JSON.stringify(pluginJson, null, 2) + "\n");
  fs.writeFileSync(path.join(dir, "index.mjs"), spec.entry);
  return dir;
}

function pack(spec, dir) {
  const zip = zipDirectory(dir);
  const meta = buildMetaFromZip(zip, {
    id: spec.id,
    version: spec.version,
    name: spec.name,
    name_zh: spec.name_zh,
    type: spec.type,
    description: spec.description,
    publisher: spec.publisher,
    channel: spec.channel,
    sandbox: { permissions: ["gui.api", "cli"] },
    engines: { node: ">=18" },
  });
  return { zip, meta };
}

async function main() {
  const opts = parseArgs(process.argv.slice(2));
  fs.mkdirSync(FIXTURES, { recursive: true });

  const dataDir = path.resolve(opts.data || DEFAULT_DATA);
  if (opts.clean && !opts.url) {
    fs.rmSync(dataDir, { recursive: true, force: true });
    console.log(`[seed] cleaned ${dataDir}`);
  }

  const store = opts.url ? null : createStore(dataDir);
  if (store) store.ensure();

  if (opts.url) {
    process.env.WIPARSE_MARKETPLACE_ALLOW_HTTP = "1";
    console.log(`[seed] publishing to ${opts.url}`);
  } else {
    console.log(`[seed] writing into data dir ${dataDir}`);
  }

  const published = [];
  for (const spec of PLUGINS) {
    const dir = writeFixture(spec);
    const { zip, meta } = pack(spec, dir);
    if (opts.url) {
      const token =
        opts.token ||
        String(process.env.MARKETPLACE_PUBLISH_TOKENS || "dev-token")
          .split(/[,;]/)[0]
          .trim();
      const client = createMarketplaceClient({
        baseUrl: opts.url,
        token,
        allowHttp: true,
      });
      await client.publish(spec.id, {
        meta,
        artifactBase64: zip.toString("base64"),
      });
      published.push(`${spec.id}@${spec.version}`);
      console.log(`  + ${spec.id}@${spec.version} (${zip.length} bytes)`);
    } else {
      store.putVersion(meta, zip);
      published.push(`${spec.id}@${spec.version}`);
      console.log(
        `  + ${spec.id}@${spec.version} sha=${String(meta.sha256).slice(0, 12)}…`
      );
    }
  }

  if (!opts.url) {
    const catalog = store.listCatalog({ channel: "stable" });
    console.log(`[seed] catalog entries=${catalog.length}`);
    console.log(
      JSON.stringify({ ok: true, plugins: catalog.map((p) => p.id) }, null, 2)
    );
  } else {
    console.log(JSON.stringify({ ok: true, published }, null, 2));
  }
}

main().catch((e) => {
  console.error("[seed] failed:", e);
  process.exit(1);
});
