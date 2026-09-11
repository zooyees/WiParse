import test from "node:test";
import assert from "node:assert/strict";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { createApp } from "../src/server.mjs";
import {
  zipDirectory,
  buildMetaFromZip,
} from "../../../test-tools/lib/marketplace-install.mjs";
import { createMarketplaceClient } from "../../../test-tools/lib/marketplace-client.mjs";
import { installFromZip } from "../../../test-tools/lib/marketplace-install.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

function makePluginDir() {
  const dir = fs.mkdtempSync(path.join(os.tmpdir(), "mkt-plug-"));
  fs.writeFileSync(
    path.join(dir, "plugin.json"),
    JSON.stringify({
      id: "cloud-demo",
      name: "Cloud Demo",
      type: "smoke",
      version: "2.0.0",
      entry: "index.mjs",
      capabilities: ["run"],
      publisher: "wiparse",
    })
  );
  fs.writeFileSync(
    path.join(dir, "index.mjs"),
    "export default async function run(){ return { ok: true }; }\n"
  );
  return dir;
}

test("health catalog publish download authz", async () => {
  const dataDir = fs.mkdtempSync(path.join(os.tmpdir(), "mkt-data-"));
  const pluginDir = makePluginDir();
  const zip = zipDirectory(pluginDir);
  const meta = buildMetaFromZip(zip, {
    id: "cloud-demo",
    version: "2.0.0",
    name: "Cloud Demo",
    type: "smoke",
    publisher: "wiparse",
    channel: "stable",
  });

  const app = createApp({ dataDir, publishTokens: ["secret-token"] });
  const { url, close } = await app.listen(0, "127.0.0.1");
  try {
    process.env.WIPARSE_MARKETPLACE_ALLOW_HTTP = "1";
    const anon = createMarketplaceClient({ baseUrl: url, allowHttp: true });
    const health = await anon.health();
    assert.equal(health.ok, true);

    await assert.rejects(
      () =>
        anon.publish("cloud-demo", {
          meta,
          artifactBase64: zip.toString("base64"),
        }),
      (e) => e.code === "E_AUTH"
    );

    const authed = createMarketplaceClient({
      baseUrl: url,
      token: "secret-token",
      allowHttp: true,
    });
    const pub = await authed.publish("cloud-demo", {
      meta,
      artifactBase64: zip.toString("base64"),
    });
    assert.equal(pub.ok, true);
    assert.equal(pub.version.version, "2.0.0");

    const catalog = await anon.catalog({ channel: "stable" });
    assert.equal(catalog.ok, true);
    assert.equal(catalog.plugins.length, 1);
    assert.equal(catalog.plugins[0].id, "cloud-demo");

    const ver = await anon.getVersion("cloud-demo", "2.0.0");
    assert.equal(ver.version.sha256, meta.sha256);

    const bytes = await anon.download("cloud-demo", "2.0.0");
    assert.equal(bytes.length, zip.length);

    const installRoot = fs.mkdtempSync(path.join(os.tmpdir(), "mkt-inst-"));
    try {
      const installed = installFromZip({
        marketplaceRoot: installRoot,
        meta: ver.version,
        zipBytes: bytes,
        trust: { allowed_publishers: ["wiparse"] },
        source: url,
      });
      assert.equal(installed.ok, true);
      assert.ok(fs.existsSync(path.join(installed.dir, "plugin.json")));
    } finally {
      fs.rmSync(installRoot, { recursive: true, force: true });
    }

    await authed.unpublish("cloud-demo", "2.0.0");
    const catalog2 = await anon.catalog();
    assert.equal(catalog2.plugins.length, 0);
  } finally {
    await close();
    fs.rmSync(dataDir, { recursive: true, force: true });
    fs.rmSync(pluginDir, { recursive: true, force: true });
    delete process.env.WIPARSE_MARKETPLACE_ALLOW_HTTP;
  }
});
