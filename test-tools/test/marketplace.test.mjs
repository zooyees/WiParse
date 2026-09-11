import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  generateEd25519KeyPair,
  signSha256Hex,
  verifyArtifactIntegrity,
  MarketplaceError,
  sha256Hex,
  validatePackageMeta,
} from "../lib/marketplace-trust.mjs";
import {
  resolveMarketplaceRoot,
  listInstalled,
  ensureMarketplaceLayout,
} from "../lib/marketplace-registry.mjs";
import {
  zipDirectory,
  installFromZip,
  buildMetaFromZip,
} from "../lib/marketplace-install.mjs";
import { discoverPluginsMerged } from "../runner.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const FIXTURE_PLUGIN = path.join(__dirname, "fixtures", "marketplace", "demo-plugin");

function writeFixturePlugin() {
  fs.mkdirSync(FIXTURE_PLUGIN, { recursive: true });
  fs.writeFileSync(
    path.join(FIXTURE_PLUGIN, "plugin.json"),
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
    path.join(FIXTURE_PLUGIN, "index.mjs"),
    `export default async function run(ctx) {
  return { ok: true, plugin: "demo-marketplace-plugin", lifecycle: ctx.lifecycle };
}
export async function preflight(ctx) {
  return { ok: true, lifecycle: "preflight", checks: [{ id: "smoke", ok: true }] };
}
`
  );
}

test("validate package meta + install/list/discover", async () => {
  writeFixturePlugin();
  const zip = zipDirectory(FIXTURE_PLUGIN);
  const meta = buildMetaFromZip(zip, {
    id: "demo-marketplace-plugin",
    version: "1.0.0",
    name: "Demo Marketplace Plugin",
    type: "smoke",
    publisher: "wiparse",
    channel: "stable",
    sandbox: { permissions: ["gui.api", "cli"] },
  });
  assert.equal(validatePackageMeta(meta).ok, true);
  verifyArtifactIntegrity({ meta, bytes: zip });

  const root = fs.mkdtempSync(path.join(os.tmpdir(), "wiparse-mkt-"));
  try {
    const result = installFromZip({
      marketplaceRoot: root,
      meta,
      zipBytes: zip,
      trust: { allowed_publishers: ["wiparse"] },
      source: "test",
    });
    assert.equal(result.ok, true);
    assert.ok(fs.existsSync(path.join(result.dir, "plugin.json")));
    const listed = listInstalled(root);
    assert.equal(listed.length, 1);
    assert.equal(listed[0].id, "demo-marketplace-plugin");
    assert.equal(listed[0].active, "1.0.0");

    const bundled = path.join(__dirname, "..", "plugins");
    const merged = discoverPluginsMerged({
      pluginsRoot: bundled,
      marketplaceRoot: root,
    });
    const demo = merged.find((p) => p.id === "demo-marketplace-plugin");
    assert.ok(demo);
    assert.equal(demo.source, "marketplace");
  } finally {
    fs.rmSync(root, { recursive: true, force: true });
  }
});

test("hash mismatch fails with E_HASH", () => {
  writeFixturePlugin();
  const zip = zipDirectory(FIXTURE_PLUGIN);
  const meta = buildMetaFromZip(zip, {
    id: "demo-marketplace-plugin",
    version: "1.0.0",
    publisher: "wiparse",
  });
  meta.sha256 = "0".repeat(64);
  assert.throws(
    () => verifyArtifactIntegrity({ meta, bytes: zip }),
    (e) => e instanceof MarketplaceError && e.code === "E_HASH"
  );
});

test("signature required + verify", () => {
  writeFixturePlugin();
  const zip = zipDirectory(FIXTURE_PLUGIN);
  const keys = generateEd25519KeyPair();
  const meta = buildMetaFromZip(zip, {
    id: "demo-marketplace-plugin",
    version: "1.0.0",
    publisher: "wiparse",
  });
  meta.signature = signSha256Hex(meta.sha256, keys.privateKeyPem);
  verifyArtifactIntegrity({
    meta,
    bytes: zip,
    trust: {
      require_signature: true,
      public_keys: [keys.publicKeyPem],
      allowed_publishers: ["wiparse"],
    },
  });
  assert.throws(
    () =>
      verifyArtifactIntegrity({
        meta: { ...meta, signature: null },
        bytes: zip,
        trust: { require_signature: true, public_keys: [keys.publicKeyPem] },
      }),
    (e) => e.code === "E_SIG"
  );
});

test("path escape rejected on zip entry", async () => {
  // Craft a zip with ".." name via install helpers is hard; unit the trust layout instead.
  const { unzipToDirectory } = await import("../lib/marketplace-install.mjs");
  // Minimal zip local header with name ../evil
  const name = Buffer.from("../evil.txt");
  const data = Buffer.from("x");
  const local = Buffer.alloc(30);
  local.writeUInt32LE(0x04034b50, 0);
  local.writeUInt16LE(0, 8);
  local.writeUInt32LE(data.length, 18);
  local.writeUInt32LE(data.length, 22);
  local.writeUInt16LE(name.length, 26);
  const zip = Buffer.concat([local, name, data]);
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "wiparse-zip-"));
  try {
    assert.throws(
      () => unzipToDirectory(zip, dir),
      (e) => e.code === "E_LAYOUT"
    );
  } finally {
    fs.rmSync(dir, { recursive: true, force: true });
  }
});

test("resolveMarketplaceRoot defaults under data root", () => {
  const r = resolveMarketplaceRoot({ dataRoot: "/tmp/data" });
  assert.equal(r, path.resolve("/tmp/data/marketplace"));
  ensureMarketplaceLayout; // touch import
  assert.equal(sha256Hex(Buffer.from("a")).length, 64);
});
