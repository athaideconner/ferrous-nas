# Architecture

FerrousNAS is split into a **control plane** (the Rust daemon) and a
**dashboard** (the React SPA). This is the same shape as TrueNAS (a Python
middleware + a web UI) and CasaOS (a Go daemon + a Vue UI).

## Components

### `ferrous-nasd` (backend/)

An async [axum](https://github.com/tokio-rs/axum) HTTP server.

```
src/
  main.rs        wiring: router, CORS, static file serving, bind
  models.rs      all domain types + request payloads (serde)
  error.rs       ApiError -> JSON { "error": ... }
  state.rs       Store: the in-memory "database", seeded on boot
  api/
    system.rs    /system, /system/stats, power, /alerts
    storage.rs   /storage/disks, /pools, /datasets  (+ create/delete/scrub)
    shares.rs    /shares  (+ create/patch/delete)
    apps.rs      /apps, /apps/catalog  (+ install/start/stop/uninstall)
    users.rs     /users, /groups
    network.rs   /network/interfaces
```

**State model.** Everything lives in a single `Store` guarded by a
`tokio::sync::RwLock`, shared as `Arc<RwLock<Store>>`. Read handlers take a read
lock; mutating handlers take a write lock. On boot, `Store::seeded()` fills it
with believable data (6 disks, 2 pools, 5 datasets, 4 shares, a 12-app catalog,
users, interfaces, alerts). Mutations persist for the life of the process —
create a pool and it shows up everywhere until restart.

Why in-memory? It keeps the mock honest and dependency-free: the whole system is
observable and resettable, and there's exactly one place (`state.rs`) that a
real implementation would replace.

### Dashboard (frontend/)

React + TypeScript + Vite, no UI framework — a small hand-rolled component set.

```
src/
  main.tsx           mounts <App/> under a BrowserRouter
  App.tsx            sidebar + top bar + routes
  styles.css         the entire design system (dark, rust-orange accent)
  lib/api.ts         typed fetch client + formatting helpers
  lib/hooks.ts       useAsync(fn, intervalMs?) — fetch + optional live polling
  components/ui.tsx   Ring, Meter, Sparkline, Badge, Toggle, Modal, toasts, Async
  pages/*            one file per nav item
```

Live pages (Dashboard, Apps, Network) poll on an interval; the poll refetches
*silently* so the UI never flickers.

## From mock to real

Each domain is a thin handler over `Store`. To make one real, replace the
`Store` reads/writes for that domain with calls to the OS, keeping the same API
shape so the dashboard is unchanged:

| Domain | Real backing |
|--------|--------------|
| system / stats | `/proc`, `sysinfo`, `sysfs` hwmon for temps |
| disks / S.M.A.R.T. | `lsblk --json`, `smartctl --json` |
| pools / datasets | `zfs`/`zpool` (or `mdadm` + `btrfs`) via a command runner |
| shares | render `/etc/samba/smb.conf` + `/etc/exports`, reload `smbd`/`nfsd` |
| apps | the Docker Engine API (`/var/run/docker.sock`) |
| users / groups | `useradd`/`smbpasswd`, or PAM |
| power | `systemctl reboot` / `poweroff` |

A clean way to stage this: put a `trait StorageBackend` (etc.) behind the
handlers, with a `MockBackend` (today) and a `ZfsBackend` (later), chosen by an
env flag. That preserves the "all mocked" default while letting a real backend
opt in per subsystem.

## API

Base path: `/api/v1`. All responses are JSON; errors are `{ "error": "..." }`
with an appropriate status.

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/system` | host info, CPU, memory, load |
| GET | `/system/stats?points=N` | rolling time-series for charts |
| POST | `/system/reboot`, `/system/shutdown` | mock power actions |
| GET | `/alerts` · POST `/alerts/:id/ack` | notifications |
| GET | `/storage/disks` | physical disks + S.M.A.R.T. |
| GET/POST | `/storage/pools` · DELETE `/storage/pools/:id` | pools |
| POST | `/storage/pools/:id/scrub` | start a scrub |
| GET/POST | `/storage/datasets` · DELETE `/storage/datasets/:id` | datasets |
| GET/POST | `/shares` · PATCH/DELETE `/shares/:id` | SMB/NFS shares |
| GET | `/apps/catalog` | the app store |
| GET/POST | `/apps` · DELETE `/apps/:id` | installed apps |
| POST | `/apps/:id/start`, `/apps/:id/stop` | lifecycle |
| GET/POST | `/users` · DELETE `/users/:id` · GET `/groups` | accounts |
| GET | `/network/interfaces` | NICs |
| GET | `/healthz` | liveness |

## Security note

For the mock, CORS is fully permissive and there is no auth — it's meant to run
on `localhost`. A real deployment must add authentication (session or token),
lock CORS to the dashboard origin, and run the daemon behind TLS.
