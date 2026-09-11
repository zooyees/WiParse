#!/usr/bin/env bash
# Deploy local Testing Hub marketplace server with seeded sample plugins.
#
#   ./scripts/deploy-marketplace.sh
#   PORT=8787 TOKEN=dev-token ./scripts/deploy-marketplace.sh
#   ./scripts/deploy-marketplace.sh --foreground
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
PORT="${PORT:-8787}"
HOST="${HOST:-127.0.0.1}"
TOKEN="${TOKEN:-dev-token}"
DATA_DIR="${DATA_DIR:-$ROOT/services/testing-hub-marketplace/data}"
PID_FILE="${PID_FILE:-/tmp/wiparse-marketplace.pid}"
LOG_FILE="${LOG_FILE:-/tmp/wiparse-marketplace.log}"
FOREGROUND=0

for arg in "$@"; do
  case "$arg" in
    --foreground|-f) FOREGROUND=1 ;;
    --help|-h)
      echo "Usage: $0 [--foreground]"
      exit 0
      ;;
  esac
done

cd "$ROOT"

echo "[deploy] seeding plugins → $DATA_DIR"
CLEAN=1 node "$ROOT/scripts/seed-marketplace-plugins.mjs" --data "$DATA_DIR"

export MARKETPLACE_PUBLISH_TOKENS="$TOKEN"
export MARKETPLACE_DATA_DIR="$DATA_DIR"
export PORT HOST

if [[ -f "$PID_FILE" ]] && kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
  echo "[deploy] stopping previous server pid=$(cat "$PID_FILE")"
  kill "$(cat "$PID_FILE")" 2>/dev/null || true
  sleep 0.5
  rm -f "$PID_FILE"
fi

if command -v fuser >/dev/null 2>&1; then
  fuser -k "${PORT}/tcp" >/dev/null 2>&1 || true
fi

CMD=(node "$ROOT/services/testing-hub-marketplace/src/index.mjs" --port "$PORT" --host "$HOST" --data "$DATA_DIR")

if [[ "$FOREGROUND" -eq 1 ]]; then
  echo "[deploy] listening http://$HOST:$PORT (token=$TOKEN) foreground"
  exec env MARKETPLACE_PUBLISH_TOKENS="$TOKEN" MARKETPLACE_DATA_DIR="$DATA_DIR" "${CMD[@]}"
fi

nohup env MARKETPLACE_PUBLISH_TOKENS="$TOKEN" MARKETPLACE_DATA_DIR="$DATA_DIR" \
  "${CMD[@]}" >"$LOG_FILE" 2>&1 &
echo $! >"$PID_FILE"
sleep 0.6

if ! kill -0 "$(cat "$PID_FILE")" 2>/dev/null; then
  echo "[deploy] server failed to start; see $LOG_FILE"
  cat "$LOG_FILE" || true
  exit 1
fi

echo "[deploy] server pid=$(cat "$PID_FILE") url=http://$HOST:$PORT log=$LOG_FILE token=$TOKEN"
curl -fsS "http://$HOST:$PORT/v1/health" || true
echo
curl -fsS "http://$HOST:$PORT/v1/catalog?channel=stable" || true
echo
