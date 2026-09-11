#!/usr/bin/env node
/**
 * Testing Hub marketplace cloud server.
 *
 *   node src/index.mjs [--port 8787] [--data ./data]
 *
 * Env:
 *   PORT, HOST, MARKETPLACE_DATA_DIR
 *   MARKETPLACE_PUBLISH_TOKENS  (comma-separated bearer tokens)
 */

import path from "node:path";
import { fileURLToPath } from "node:url";
import { createApp } from "./server.mjs";

const __dirname = path.dirname(fileURLToPath(import.meta.url));

function parseArgs(argv) {
  const out = { port: null, host: null, data: null };
  for (let i = 0; i < argv.length; i++) {
    const a = argv[i];
    if (a === "--port" && argv[i + 1]) out.port = Number(argv[++i]);
    else if (a === "--host" && argv[i + 1]) out.host = argv[++i];
    else if (a === "--data" && argv[i + 1]) out.data = argv[++i];
  }
  return out;
}

const opts = parseArgs(process.argv.slice(2));
const port = Number(opts.port || process.env.PORT || 8787);
const host = opts.host || process.env.HOST || "127.0.0.1";
const dataDir =
  opts.data ||
  process.env.MARKETPLACE_DATA_DIR ||
  path.join(__dirname, "..", "data");
const tokens = String(process.env.MARKETPLACE_PUBLISH_TOKENS || "")
  .split(/[,;]/)
  .map((s) => s.trim())
  .filter(Boolean);

const app = createApp({ dataDir, publishTokens: tokens });
const { url } = await app.listen(port, host);
console.log(
  `[testing-hub-marketplace] listening ${url} data=${path.resolve(dataDir)} tokens=${tokens.length}`
);
