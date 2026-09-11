# 🗄️ FerrousNAS

A NAS operating-system control plane in the spirit of **TrueNAS** and **CasaOS** —
a Rust system daemon plus a React dashboard for managing storage pools, SMB/NFS
shares, a Docker-style app store, users, and system health.

> **Mocked by default, real when you ask.** Out of the box no disks are
> partitioned, no services reconfigured and no containers launched — the daemon
> serves believable, stateful sample data so you can explore the whole
> experience safely on any machine. Each subsystem can then be switched to a
> real implementation independently (telemetry, apps, shares, pools — see
> [Configuration](#configuration)). **Authentication is the exception: it is on
> by default.**

<br>

## What's inside

| Area | What it does (mocked) |
|------|------------------------|
| **Dashboard** | Live CPU / memory rings, throughput sparklines, pool capacity, alerts |
| **Storage** | Physical disks with S.M.A.R.T. status, create/destroy pools (stripe/mirror/raidz1/raidz2), datasets with quotas & compression, scrub |
| **Shares** | SMB & NFS shares bound to datasets, enable/disable, read-only/guest access |
| **Apps** | A 12-app catalog (Jellyfin, Nextcloud, Immich, Pi-hole…) with install / start / stop / uninstall |
| **Users** | Users & groups, admin roles |
| **Network** | Interfaces with addresses and traffic counters |
| **System** | Host info, simulated power actions, acknowledge-able notifications |

## Authentication

Multi-user, **on by default**, with no default password. On first run the
dashboard shows a setup screen to create the initial administrator.

- **Sessions, not tokens** — a 256-bit opaque session id in an `HttpOnly`,
  `SameSite=Strict` cookie: unreadable by JavaScript and revocable server-side.
  Sessions live in memory, so a restart signs everyone out.
- **Argon2id** password hashing (OWASP defaults), stored in PHC format in
  `auth.json` at mode `0600`.
- **Two roles** — any signed-in user can read; every mutating endpoint requires
  an administrator.
- **Login throttling** with exponential backoff, and identical errors for an
  unknown user and a wrong password so accounts can't be enumerated.
- **Disabling auth restricts you to loopback.** `FERROUS_AUTH=off` is honoured
  only when bound to a loopback address; otherwise the daemon refuses to start.
  An unauthenticated FerrousNAS therefore cannot be exposed to a network.

The session cookie is marked `Secure` automatically whenever TLS (below) is
active, so the default configuration — auth on, TLS on — needs no manual step
to keep the cookie off the wire in plain text.

## TLS

**On by default**, terminated by the daemon itself — no reverse proxy
required. On first boot it generates a self-signed certificate covering
`localhost`, the machine's hostname, and its detected LAN IP, and reuses it on
every restart (so a browser exception you grant it stays valid).

- **Zero-config HTTPS.** The browser will show a one-time self-signed warning;
  everything after that — passwords, session cookies — is encrypted.
- **Bring your own certificate** by setting both `FERROUS_TLS_CERT` and
  `FERROUS_TLS_KEY` (e.g. a Let's Encrypt or internal-CA cert) — the daemon
  then skips self-signed generation entirely.
- **Terminate TLS at a reverse proxy instead** with `FERROUS_TLS=off`. In that
  case set `FERROUS_COOKIE_SECURE=1` yourself — the daemon can't tell the proxy
  is handling HTTPS on its behalf, so this doesn't happen automatically.
- Local development (`scripts/dev.sh`) runs the backend with `FERROUS_TLS=off`:
  the split Vite-proxy setup puts the browser's own connection on plain HTTP,
  and browsers only honour `Secure` cookies over their *own* HTTPS connection
  — see the comment in `frontend/vite.config.ts`. This doesn't affect
  production, where the daemon serves the built dashboard itself and
  everything is one HTTPS origin.

## Architecture

```
┌──────────────────┐        HTTP / JSON         ┌───────────────────────┐
│  React dashboard │  ───────────────────────▶  │  ferrous-nasd (Rust)  │
│  (Vite + TS)     │      /api/v1/...           │  axum + in-memory DB   │
└──────────────────┘  ◀───────────────────────  └───────────────────────┘
        served as static files by the daemon in production
```

- **backend/** — `ferrous-nasd`, an [axum](https://github.com/tokio-rs/axum) daemon.
  All state lives in memory behind an `RwLock` and is seeded on boot
  ([backend/src/state.rs](backend/src/state.rs)).
- **frontend/** — a React + TypeScript SPA (Vite). Typed API client in
  [frontend/src/lib/api.ts](frontend/src/lib/api.ts).
- **os-image/** — `mkosi` config to bake a bootable Debian appliance image.
- **scripts/** — build, dev, and systemd install helpers.

## Quick start (development)

Requires **Rust** (`rustup`) and **Node 18+**.

```bash
# terminal 1 — API on :4200, dashboard dev server on :5173 (proxies /api)
./scripts/dev.sh
# then open http://localhost:5173
```

Or run the two halves yourself (note the `FERROUS_TLS=off` — see
[TLS](#tls) above for why the split dev setup needs it):

```bash
cd backend && FERROUS_TLS=off cargo run     # http://localhost:4200
cd frontend && npm install && npm run dev   # http://localhost:5173
```

## Production build (single deployable)

```bash
./scripts/build.sh
FERROUS_WEB_DIR=frontend/dist backend/target/release/ferrous-nasd
# open https://localhost:4200 — click through the self-signed warning once
# (daemon serves the built dashboard; see TLS above to use a real certificate)
```

Install as a systemd service on a Debian/Ubuntu host:

```bash
sudo ./scripts/install.sh
```

## Bootable appliance image

`os-image/` contains a working `mkosi` definition that produces a bootable
Debian-based image with the daemon baked in. It needs a real Linux host (root +
loopback), so it can't be built inside restricted sandboxes — see
[docs/OS-IMAGE.md](docs/OS-IMAGE.md) for the full walkthrough and why a full ISO
is a host-level task rather than something this repo builds on its own.

## Configuration

| Env var | Default | Meaning |
|---------|---------|---------|
| `FERROUS_ADDR` | `0.0.0.0:4200` | Address the daemon binds |
| `FERROUS_AUTH` | _(unset → **on**)_ | Set to `off` to disable authentication. The daemon then refuses to bind anything but loopback. |
| `FERROUS_STATE_DIR` | `/var/lib/ferrous-nas` | Where `auth.json` and the self-signed TLS cert/key are kept |
| `FERROUS_COOKIE_SECURE` | _(auto)_ | Forced to `1` whenever built-in TLS is active. Set to `1` yourself when terminating TLS at a reverse proxy (`FERROUS_TLS=off`). |
| `FERROUS_TLS` | _(unset → **on**)_ | Set to `off` to serve plain HTTP (e.g. behind a reverse proxy that terminates TLS itself) |
| `FERROUS_TLS_CERT` / `FERROUS_TLS_KEY` | _(unset → self-signed)_ | Paths to a real certificate/key. Both or neither — a self-signed pair is generated into `FERROUS_STATE_DIR` when neither is set. |
| `FERROUS_WEB_DIR` | `../frontend/dist` | Where to serve the built dashboard from |
| `FERROUS_TELEMETRY` | _(unset → mock)_ | Set to `linux` (or `real`) to serve **real, read-only** host telemetry — system stats and disks from `/proc`, `sysfs`, `lsblk` and `smartctl`. Everything else stays mocked. |
| `FERROUS_APPS` | _(unset → mock)_ | Set to `docker` (or `real`) to manage **real containers** via the Docker Engine socket (pull/create/start/stop/remove). Falls back to mock if Docker isn't reachable. |
| `FERROUS_SHARES` | _(unset → mock)_ | Set to `linux` (or `real`) to render **real SMB/NFS config** and reload the services. Falls back to mock if the config files aren't writable. |
| `FERROUS_SMB_CONF` | `/etc/samba/ferrousnas-shares.conf` | Managed Samba fragment to write |
| `FERROUS_NFS_EXPORTS` | `/etc/exports.d/ferrousnas.exports` | Managed NFS exports drop-in to write |
| `FERROUS_SHARES_RELOAD` | `1` | Set to `0` to render share config without reloading `smbd`/`exportfs` |
| `FERROUS_POOLS` | _(unset → mock)_ | Set to `zfs` (or `real`) to drive real `zpool`/`zfs`. **Dry-run unless the next variable is also set.** |
| `FERROUS_POOLS_DESTRUCTIVE` | _(unset → dry-run)_ | Must be exactly `i-understand` to actually execute `zpool create`, `zpool destroy` and `zfs destroy` |
| `RUST_LOG` | `info` | Log level |

> **Real telemetry.** `FERROUS_TELEMETRY=linux cargo run` switches system
> info, live stats and the disk inventory to the actual host (read-only — it
> never writes anything). Temperatures/S.M.A.R.T. degrade gracefully when
> sensors aren't present or `smartctl` lacks root. The dashboard's top bar
> shows which source is active. Pools, shares and users remain mocked.

> **Real apps.** `FERROUS_APPS=docker cargo run` manages actual containers
> through the Docker Engine socket. FerrousNAS only ever touches containers it
> created (tagged `com.ferrousnas.*` and named `ferrousnas-<app>`), so it never
> disturbs unrelated containers. Install pulls the image, creates the container
> with the app's port published, and starts it. Requires access to
> `/var/run/docker.sock`; falls back to mock if the daemon isn't reachable.

> **Real shares.** `FERROUS_SHARES=linux` renders every enabled share to real
> Samba/NFS config and reloads the services. FerrousNAS **never edits
> `smb.conf` or `/etc/exports` in place** — it owns two managed files only: a
> Samba fragment (activate it by adding `include = /etc/samba/ferrousnas-shares.conf`
> under `[global]`) and an `/etc/exports.d` drop-in, which `exportfs` picks up
> automatically. Files are written atomically and regenerated in full from the
> API state on every change. Point `FERROUS_SMB_CONF`/`FERROUS_NFS_EXPORTS` at
> a scratch directory with `FERROUS_SHARES_RELOAD=0` to preview the output
> safely.

> ⚠️ **Real pools — this tier can destroy data.** `FERROUS_POOLS=zfs` drives
> real `zpool`/`zfs`, and is deliberately harder to arm than the others:
>
> - **Dry-run by default.** Every validation and disk safety check runs, then
>   destructive commands are refused with `403` reporting the exact argv.
>   Actually executing `zpool create` / `zpool destroy` / `zfs destroy`
>   additionally requires `FERROUS_POOLS_DESTRUCTIVE=i-understand`.
> - **Disks must prove they're empty.** Any candidate with a partition table,
>   filesystem signature, child partition, active mount, or that backs `/` is
>   refused before a single command runs.
> - **No `-f`.** ZFS's own safety checks are never forced past.
> - **No shell, and every identifier is validated** — a pool named `-f` is
>   rejected rather than reaching the CLI as a flag.
>
> Non-destructive operations (list, scrub, dataset create) run normally.

## API

Base path `\/api/v1`. Full list in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#api).
Quick taste (`-k` skips verification of the self-signed cert; drop it once
you've supplied a real one, and add `-b cookies.txt -c cookies.txt` to carry a
session across calls once auth is set up):

```bash
curl -k https://localhost:4200/api/v1/system
curl -k https://localhost:4200/api/v1/storage/pools
curl -k -X POST https://localhost:4200/api/v1/apps -d '{"catalog_id":"grafana"}' -H 'content-type: application/json'
```

## License

MIT.
