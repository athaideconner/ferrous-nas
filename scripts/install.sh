#!/usr/bin/env bash
# Install a built FerrousNAS onto a Debian/Ubuntu host as a systemd service.
# Run as root AFTER scripts/build.sh. This deploys the mock control plane; it
# does not reconfigure your real Samba/NFS/Docker.
set -euo pipefail
cd "$(dirname "$0")/.."

if [ "$(id -u)" -ne 0 ]; then echo "Run as root (sudo)."; exit 1; fi

BIN=backend/target/release/ferrous-nasd
[ -f "$BIN" ] || { echo "Missing $BIN — run scripts/build.sh first."; exit 1; }
[ -d frontend/dist ] || { echo "Missing frontend/dist — run scripts/build.sh first."; exit 1; }

echo "==> Creating service user"
useradd --system --home /var/lib/ferrous-nas --shell /usr/sbin/nologin ferrous 2>/dev/null || true

echo "==> Installing files"
install -Dm755 "$BIN" /usr/local/bin/ferrous-nasd
rm -rf /usr/local/share/ferrous-nas/web
install -d /usr/local/share/ferrous-nas/web
cp -r frontend/dist/* /usr/local/share/ferrous-nas/web/
install -Dm644 scripts/ferrous-nasd.service /etc/systemd/system/ferrous-nasd.service

echo "==> Enabling service"
systemctl daemon-reload
systemctl enable --now ferrous-nasd.service

echo
echo "FerrousNAS is running. Browse to http://$(hostname -I | awk '{print $1}'):4200"
