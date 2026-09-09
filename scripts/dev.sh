#!/usr/bin/env bash
# Run FerrousNAS in development: the Rust API on :4200 and the Vite dev server
# on :5173 (which proxies /api to the daemon). Ctrl-C stops both.
set -euo pipefail
cd "$(dirname "$0")/.."

cleanup() { kill 0 2>/dev/null || true; }
trap cleanup EXIT INT TERM

echo "==> Starting daemon on http://localhost:4200"
( cd backend && cargo run ) &

echo "==> Starting dashboard dev server on http://localhost:5173"
( cd frontend && npm install && npm run dev )
