# 🗄️ FerrousNAS

A NAS operating-system control plane in the spirit of **TrueNAS** and **CasaOS** —
a Rust system daemon plus a React dashboard for managing storage pools, SMB/NFS
shares, a Docker-style app store, users, and system health.

> **Everything is mocked.** No real disks are partitioned, no services are
> reconfigured, no containers are launched. The daemon serves believable,
> stateful sample data so you can run and explore the whole experience safely on
> any machine. The code is structured so each mock can be swapped for a real
> implementation later (see [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md)).

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

Or run the two halves yourself:

```bash
cd backend && cargo run           # http://localhost:4200
cd frontend && npm install && npm run dev   # http://localhost:5173
```

## Production build (single deployable)

```bash
./scripts/build.sh
FERROUS_WEB_DIR=frontend/dist backend/target/release/ferrous-nasd
# open http://localhost:4200  (daemon serves the built dashboard)
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
| `FERROUS_WEB_DIR` | `../frontend/dist` | Where to serve the built dashboard from |
| `FERROUS_TELEMETRY` | _(unset → mock)_ | Set to `linux` (or `real`) to serve **real, read-only** host telemetry — system stats and disks from `/proc`, `sysfs`, `lsblk` and `smartctl`. Everything else stays mocked. |
| `RUST_LOG` | `info` | Log level |

> **Real telemetry.** `FERROUS_TELEMETRY=linux cargo run` switches system
> info, live stats and the disk inventory to the actual host (read-only — it
> never writes anything). Temperatures/S.M.A.R.T. degrade gracefully when
> sensors aren't present or `smartctl` lacks root. The dashboard's top bar
> shows which source is active. Pools, shares, apps and users remain mocked.

## API

Base path `\/api/v1`. Full list in [docs/ARCHITECTURE.md](docs/ARCHITECTURE.md#api).
Quick taste:

```bash
curl localhost:4200/api/v1/system
curl localhost:4200/api/v1/storage/pools
curl -X POST localhost:4200/api/v1/apps -d '{"catalog_id":"grafana"}' -H 'content-type: application/json'
```

## License

MIT.
