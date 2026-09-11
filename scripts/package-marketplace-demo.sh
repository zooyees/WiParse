#!/usr/bin/env bash
# Build WiParse (Linux) + marketplace demo bundle for local testing.
#
#   ./scripts/package-marketplace-demo.sh
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
OUT="${OUT:-$ROOT/dist/wiparse-linux-marketplace-demo}"
ARCHIVE="${ARCHIVE:-$ROOT/dist/wiparse-linux-marketplace-demo.tar.gz}"
TOKEN="${TOKEN:-dev-token}"
PORT="${PORT:-8787}"

cd "$ROOT"

echo "[package] cargo release build"
cargo build --release -p wiparse-gui -p wiparse-cli --locked

echo "[package] seed marketplace data"
CLEAN=1 node "$ROOT/scripts/seed-marketplace-plugins.mjs" \
  --data "$ROOT/services/testing-hub-marketplace/data"

rm -rf "$OUT"
mkdir -p "$OUT"/{bin,test-tools,services,scripts,marketplace}

cp -a "$ROOT/target/release/wiparse-gui" "$OUT/bin/wiparse-gui"
cp -a "$ROOT/target/release/wiparse" "$OUT/bin/wiparse"

# Runtime deps (prefer cp -a tree; rsync optional)
copy_tree() {
  local src="$1" dst="$2"
  mkdir -p "$dst"
  if command -v rsync >/dev/null 2>&1; then
    rsync -a --delete \
      --exclude '**/logs/' \
      --exclude '**/node_modules/' \
      --exclude '**/_loop_*' \
      "$src" "$dst"
  else
    cp -a "$src"/. "$dst"/
  fi
}

copy_tree "$ROOT/test-tools" "$OUT/test-tools"
copy_tree "$ROOT/services/testing-hub-marketplace" "$OUT/services/testing-hub-marketplace"

cp -a "$ROOT/scripts/deploy-marketplace.sh" "$OUT/scripts/"
cp -a "$ROOT/scripts/seed-marketplace-plugins.mjs" "$OUT/scripts/"
cp -a "$ROOT/scripts/local-marketplace-sim.mjs" "$OUT/scripts/"

cat > "$OUT/config.json" <<EOF
{
  "ui": {
    "language": "zh",
    "theme": "dark"
  },
  "apps": {
    "test_tool": {
      "plugins_dir": "test-tools/plugins",
      "cli_path": "bin/wiparse",
      "node_path": "node",
      "data_root": ".",
      "marketplace": {
        "enabled": true,
        "base_url": "http://127.0.0.1:${PORT}",
        "install_dir": "marketplace",
        "channel": "stable"
      }
    }
  }
}
EOF

cat > "$OUT/README-MARKETPLACE-DEMO.md" <<EOF
# WiParse Testing Hub Marketplace Demo (Linux)

## 1. Start marketplace server

\`\`\`bash
./start-marketplace.sh
\`\`\`

- URL: http://127.0.0.1:${PORT}
- Publish token: ${TOKEN}

Seeded plugins:
- demo-marketplace-plugin@1.0.0
- market-echo@1.0.0
- market-counter@1.1.0
- market-preflight-lab@0.9.0

## 2. Launch WiParse

\`\`\`bash
./start-wiparse.sh
\`\`\`

In **集成测试 / Testing Hub**:
1. Switch to **市场 / Market**
2. Confirm URL \`http://127.0.0.1:${PORT}\`
3. Refresh → Install a plugin → switch back to **插件** and run it

## CLI smoke

\`\`\`bash
export WIPARSE_MARKETPLACE_ALLOW_HTTP=1
node test-tools/marketplace.mjs catalog --url http://127.0.0.1:${PORT} --json
node test-tools/marketplace.mjs pull --plugin market-echo --version 1.0.0 \\
  --url http://127.0.0.1:${PORT} --install-dir ./marketplace --json
\`\`\`
EOF

cat > "$OUT/start-marketplace.sh" <<EOF
#!/usr/bin/env bash
set -euo pipefail
cd "\$(dirname "\$0")"
export TOKEN=${TOKEN}
export PORT=${PORT}
export DATA_DIR="\$PWD/services/testing-hub-marketplace/data"
export HOST=127.0.0.1
# Redeploy from package scripts with package-local paths
ROOT="\$PWD" PORT="\$PORT" HOST="\$HOST" TOKEN="\$TOKEN" DATA_DIR="\$DATA_DIR" \\
  bash ./scripts/deploy-marketplace.sh "\$@"
EOF
chmod +x "$OUT/start-marketplace.sh" "$OUT/scripts/deploy-marketplace.sh"

cat > "$OUT/start-wiparse.sh" <<EOF
#!/usr/bin/env bash
set -euo pipefail
HERE="\$(cd "\$(dirname "\$0")" && pwd)"
cd "\$HERE"
export WIPARSE_MARKETPLACE_ALLOW_HTTP=1
export WIPARSE_PROJECT_ROOT="\$HERE"
export WIPARSE_CONFIG="\${WIPARSE_CONFIG:-\$HERE/config.json}"
exec "\$HERE/bin/wiparse-gui" "\$@"
EOF
chmod +x "$OUT/start-wiparse.sh" "$OUT/bin/"*

echo "[package] writing archive $ARCHIVE"
mkdir -p "$(dirname "$ARCHIVE")"
tar -C "$(dirname "$OUT")" -czf "$ARCHIVE" "$(basename "$OUT")"
echo "[package] done → $OUT"
echo "[package] archive → $ARCHIVE"
du -sh "$OUT" "$ARCHIVE"
