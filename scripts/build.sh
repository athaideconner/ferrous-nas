#!/usr/bin/env bash
# Build the FerrousNAS dashboard and the release daemon binary.
# The daemon serves the built dashboard, so this yields a single deployable.
set -euo pipefail
cd "$(dirname "$0")/.."

echo "==> Building dashboard (frontend/)"
( cd frontend && npm install && npm run build )

echo "==> Building daemon (backend/, release)"
( cd backend && cargo build --release )

echo
echo "Done."
echo "  binary : backend/target/release/ferrous-nasd"
echo "  web    : frontend/dist"
echo
echo "Run it:  FERROUS_WEB_DIR=frontend/dist backend/target/release/ferrous-nasd"
echo "Then open http://localhost:4200"
