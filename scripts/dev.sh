#!/usr/bin/env bash
# Run FerrousNAS in development: the Rust API on :4200 and the Vite dev server
# on :5173 (which proxies /api to the daemon). Ctrl-C stops both.
#
# The backend runs with FERROUS_TLS=off here. The daemon serves HTTPS by
# default, but a browser only honours the session cookie's `Secure` flag when
# its own connection is HTTPS — the split dev setup has the browser talking to
# Vite's plain-http origin (:5173), which proxies to the backend, so a Secure
# cookie set by an HTTPS backend would silently fail to persist through it.
# Production has no such issue: the daemon serves the built SPA itself, so
# everything is one HTTPS origin. See docs/ARCHITECTURE.md.
set -euo pipefail
cd "$(dirname "$0")/.."

cleanup() { kill 0 2>/dev/null || true; }
trap cleanup EXIT INT TERM

echo "==> Starting daemon on http://localhost:4200 (TLS off for local dev)"
( cd backend && FERROUS_TLS=off cargo run ) &

echo "==> Starting dashboard dev server on http://localhost:5173"
( cd frontend && npm install && npm run dev )
