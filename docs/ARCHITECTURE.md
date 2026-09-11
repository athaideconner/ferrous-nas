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

## Real subsystems (implemented)

Three subsystems are already backed by real implementations, all following the
same trait-swap pattern:

- **Telemetry** (read-only) — `FERROUS_TELEMETRY=linux`. Detailed below.
- **Apps** — `FERROUS_APPS=docker`. An `AppManager` trait
  ([appmgr/mod.rs](../backend/src/appmgr/mod.rs)) with `MockAppManager`
  (default) and `DockerAppManager` ([appmgr/docker.rs](../backend/src/appmgr/docker.rs)),
  which drives the Docker Engine via [bollard](https://crates.io/crates/bollard):
  pull → create (port published, `com.ferrousnas.*` labels) → start, plus
  stop/remove. It lists only containers it manages, so it never touches
  unrelated containers.
- **Shares** — `FERROUS_SHARES=linux`. A `ShareManager` trait
  ([sharemgr/mod.rs](../backend/src/sharemgr/mod.rs)) with `MockShareManager`
  (default) and `LinuxShareManager` ([sharemgr/linux.rs](../backend/src/sharemgr/linux.rs)).
  Both share the same store-mutation helpers (`do_create`/`do_patch`/`do_delete`)
  so behaviour can't drift; the Linux one additionally re-renders the full
  share set to a managed Samba fragment and an `/etc/exports.d` drop-in
  (written atomically), then reloads `smbd` and `exportfs` best-effort. The
  store remains the source of truth — config is a pure projection of it, and
  FerrousNAS never edits `smb.conf`/`/etc/exports` in place. Renderers are
  unit-tested.

### Telemetry

The read-only telemetry subsystem shows the migration pattern. It lives behind
a trait:

```
telemetry/
  mod.rs     trait Telemetry + MockTelemetry (default) + build() selector
  linux.rs   LinuxTelemetry — reads /proc, sysfs, lsblk, smartctl (read-only)
```

`Telemetry` has three methods — `system_info`, `stats_history`, `disks` — and
the daemon picks an implementation at boot from `FERROUS_TELEMETRY`
(`linux`/`real` → real host, anything else → mock). Handlers extract the source
via `State<TelemetryRef>`; the composite [`AppState`](../backend/src/app.rs)
uses `FromRef` so DB-backed handlers keep extracting `State<Db>` unchanged.

`LinuxTelemetry` keeps a small in-memory ring buffer + previous `/proc/stat`,
`/proc/net/dev` and `/proc/diskstats` snapshots to compute CPU %, and network /
disk throughput from counter deltas. It is strictly read-only and falls back to
the mock if `/proc/stat` can't be read.

## From mock to real (remaining subsystems)

Each remaining domain is a thin handler over `Store`. To make one real, add a
trait like `Telemetry` with a `Mock*` and a real impl, keeping the API shape so
the dashboard is unchanged:

| Domain | Status | Real backing |
|--------|--------|--------------|
| system / stats | ✅ done | `/proc` (stat, meminfo, loadavg, uptime, cpuinfo), `sysfs` hwmon |
| disks / S.M.A.R.T. | ✅ done | `lsblk --json`, `smartctl --json` |
| pools / datasets | mocked | `zfs`/`zpool` (or `mdadm` + `btrfs`) via a command runner |
| shares | ✅ done | render a managed Samba fragment + `/etc/exports.d` drop-in, reload `smbd`/`exportfs` — `FERROUS_SHARES=linux` |
| apps | ✅ done | the Docker Engine API (`/var/run/docker.sock`) via bollard — `FERROUS_APPS=docker` |
| users / groups | mocked | `useradd`/`smbpasswd`, or PAM |
| power | mocked | `systemctl reboot` / `poweroff` |

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
| GET | `/system/telemetry` | active telemetry source (`mock` / `linux`) |
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
