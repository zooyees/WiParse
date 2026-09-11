#!/usr/bin/env node
/**
 * Local marketplace simulation (loopback HTTP — no external cloud).
 *
 *   node scripts/local-marketplace-sim.mjs
 *   CLEAN=1 node scripts/local-marketplace-sim.mjs
 */

import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";
import { createApp } from "../services/testing-hub-marketplace/src/server.mjs";
import {
  zipDirectory,
  buildMetaFromZip,
  installFromZip,
} from "../test-tools/lib/marketplace-install.mjs";
import { createMarketplaceClient } from "../test-tools/lib/marketplace-client.mjs";
import { listInstalled } from "../test-tools/lib/marketplace-registry.mjs";
import { discoverPluginsMerged } from "../test-tools/runner.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const ROOT = path.resolve(__dirname, "..");
const FIXTURE = path.join(
  ROOT,
  "test-tools/test/fixtures/marketplace/demo-plugin"
);
const TOKEN = "local-dev-token";
const PORT = Number(process.env.MARKETPLACE_SIM_PORT || 8787);

function ensureFixture() {
  fs.mkdirSync(FIXTURE, { recursive: true });
  fs.writeFileSync(
    path.join(FIXTURE, "plugin.json"),
    JSON.stringify(
      {
        id: "demo-marketplace-plugin",
        name: "Demo Marketplace Plugin",
        type: "smoke",
        version: "1.0.0",
        entry: "index.mjs",
        capabilities: ["preflight", "run"],
        publisher: "wiparse",
        marketplace: { channel: "stable" },
        sandbox: { permissions: ["gui.api", "cli"] },
        engines: { node: ">=18" },
      },
      null,
      2
    )
  );
  fs.writeFileSync(
    path.join(FIXTURE, "index.mjs"),
    `export default async function run(ctx) {
  return { ok: true, plugin: "demo-marketplace-plugin", lifecycle: ctx.lifecycle };
}
export async function preflight(ctx) {
  return { ok: true, lifecycle: "preflight", checks: [{ id: "smoke", ok: true }] };
}
`
  );
}

function step(title) {
  console.log(`\n==> ${title}`);
}

async function main() {
  ensureFixture();
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), "mkt-server-"));
  const installRoot = fs.mkdtempSync(path.join(os.tmpdir(), "mkt-client-"));

  const app = createApp({ dataDir, publishTokens: [TOKEN] });
  const { url, close } = await app.listen(PORT, "127.0.0.1");
  process.env.WIPARSE_MARKETPLACE_ALLOW_HTTP = "1";

  try {
    step(`Server up at ${url} (token=${TOKEN})`);

    const zip = zipDirectory(FIXTURE);
    const meta = buildMetaFromZip(zip, {
      id: "demo-marketplace-plugin",
      version: "1.0.0",
      name: "Demo Marketplace Plugin",
      type: "smoke",
      publisher: "wiparse",
      channel: "stable",
      sandbox: { permissions: ["gui.api", "cli"] },
    });
    console.log(`packed zip size=${zip.length} sha256=${meta.sha256.slice(0, 12)}…`);

    const pub = createMarketplaceClient({
      baseUrl: url,
      token: TOKEN,
      allowHttp: true,
    });
    const anon = createMarketplaceClient({ baseUrl: url, allowHttp: true });

    step("Health");
    console.log(JSON.stringify(await anon.health(), null, 2));

    step("Publish fixture");
    const published = await pub.publish("demo-marketplace-plugin", {
      meta,
      artifactBase64: zip.toString("base64"),
    });
    console.log(JSON.stringify(published, null, 2));

    step("Catalog");
    const catalog = await anon.catalog({ channel: "stable" });
    console.log(JSON.stringify(catalog, null, 2));

    step("Pull + install into local marketplace root");
    const ver = await anon.getVersion("demo-marketplace-plugin", "1.0.0");
    const bytes = await anon.download("demo-marketplace-plugin", "1.0.0");
    const installed = installFromZip({
      marketplaceRoot: installRoot,
      meta: ver.version,
      zipBytes: bytes,
      trust: { allowed_publishers: ["wiparse"] },
      source: url,
    });
    console.log(JSON.stringify(installed, null, 2));
    console.log("registry:", JSON.stringify(listInstalled(installRoot), null, 2));

    step("Runner discovery (bundled + marketplace active)");
    const merged = discoverPluginsMerged({
      pluginsRoot: path.join(ROOT, "test-tools/plugins"),
      marketplaceRoot: installRoot,
    });
    const demo = merged.find((p) => p.id === "demo-marketplace-plugin");
    console.log(
      JSON.stringify(
        {
          total: merged.length,
          demo: demo
            ? {
                id: demo.id,
                version: demo.version,
                source: demo.source,
                dir: demo.dir,
              }
            : null,
        },
        null,
        2
      )
    );

    if (!demo || demo.source !== "marketplace") {
      throw new Error("expected demo-marketplace-plugin from marketplace source");
    }

    step("Preflight via installed entry");
    const entryUrl = pathToFileURL(
      path.join(demo.dir, demo.entry || "index.mjs")
    ).href;
    const mod = await import(entryUrl);
    const pf = await mod.preflight({ lifecycle: "preflight" });
    console.log(JSON.stringify(pf, null, 2));

    console.log("\nOK — local marketplace simulation succeeded.");
    console.log(`server data: ${dataDir}`);
    console.log(`client install: ${installRoot}`);
    console.log("\nManual reuse:");
    console.log(
      `  MARKETPLACE_PUBLISH_TOKENS=${TOKEN} node services/testing-hub-marketplace/src/index.mjs --port ${PORT} --data ${dataDir}`
    );
    console.log(
      `  WIPARSE_MARKETPLACE_ALLOW_HTTP=1 node test-tools/marketplace.mjs catalog --url ${url}`
    );
  } finally {
    await close();
    if (process.env.CLEAN === "1") {
      fs.rmSync(dataDir, { recursive: true, force: true });
      fs.rmSync(installRoot, { recursive: true, force: true });
    }
  }
}

main().catch((e) => {
  console.error("[local-marketplace-sim] failed:", e);
  process.exit(1);
});
