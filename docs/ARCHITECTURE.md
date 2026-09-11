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

Four subsystems are already backed by real implementations, all following the
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
- **Pools & datasets** — `FERROUS_POOLS=zfs`. The destructive tier; see below.

### Pools: the destructive tier

`PoolManager` ([poolmgr/mod.rs](../backend/src/poolmgr/mod.rs)) with
`MockPoolManager` (default) and `ZfsPoolManager`
([poolmgr/zfs.rs](../backend/src/poolmgr/zfs.rs)). Because a bug here formats
disks, the policy lives in its own pure, unit-tested module
([poolmgr/safety.rs](../backend/src/poolmgr/safety.rs)) and the defences stack:

| Layer | What it stops |
|---|---|
| Name validation | Names must start with a letter, so `-f`/`--force` can never reach the CLI as a flag; reserved vdev keywords (`mirror`, `raidz1`, …) and shell/path metacharacters are rejected |
| argv, never a shell | No interpolation, no shell metacharacter surface |
| Device-id validation | Disk ids resolve to `[A-Za-z0-9]+` only, so nothing escapes into argv as a path |
| `check_disk_safe` | Refuses any disk with a partition table, filesystem signature, child partitions, an active mount, or that backs `/` |
| No `-f` | ZFS's own safety checks are never overridden |
| Dry-run gate | `zpool create`/`zpool destroy`/`zfs destroy` are refused with `403` + the exact argv unless `FERROUS_POOLS_DESTRUCTIVE=i-understand` |

Checks run **before** the gate, so an unsafe disk is rejected on its own merits
rather than being masked by dry-run. Non-destructive operations (list, scrub,
dataset create) are not gated. Command output parsing (`zpool list -v`,
`zfs list`) is pure and unit-tested against captured fixtures.

> Caveat: enabling this while other subsystems are mocked means dataset ids come
> from ZFS while shares reference mock dataset ids. For a coherent real system,
> enable telemetry, pools and shares together.

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
| pools / datasets | ✅ done | `zpool`/`zfs`, dry-run by default — `FERROUS_POOLS=zfs` |
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

Public (no session): `/auth/status`, `/auth/login`, `/auth/logout`, `/setup`.
Everything else requires a session; every mutating endpoint requires an admin.

| Method | Path | Purpose |
|--------|------|---------|
| GET | `/auth/status` | `auth_enabled` / `setup_required` |
| POST | `/setup` | create the first administrator (once) |
| POST | `/auth/login` · `/auth/logout` | session lifecycle |
| GET | `/auth/me` | the signed-in user |
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

## Authentication

Implemented in [auth/](../backend/src/auth/) and **enabled by default** —
unlike the subsystem backends, the safe configuration is the one you get by
doing nothing.

```
auth/
  mod.rs         AuthStore: users + Argon2id hashes on disk, sessions in memory
  password.rs    hashing/verification and password policy (pure)
  middleware.rs  require_auth layer + CurrentUser / AdminUser extractors
```

Design decisions worth knowing:

- **Router split, not a path allowlist.** Public routes (status/setup/login/
  logout) live in one router; everything else is in a second router carrying the
  `require_auth` layer. A newly added route is therefore protected by
  construction — there is no allowlist to forget to update.
- **Authenticate in middleware, authorise in handlers.** The layer only resolves
  the session and stashes the user in request extensions. Privileged handlers
  take an `AdminUser` extractor, so the requirement is visible in the signature
  and can't be silently dropped.
- **No CORS layer at all.** The dashboard is always same-origin (Vite proxies
  `/api` in dev; the daemon serves the SPA in production). A permissive policy
  would also be incompatible with credentialed cookies.
- **Fail closed.** If the credential store can't be opened, the daemon exits
  rather than starting without auth. If auth is disabled, it refuses to bind a
  non-loopback address.
- **Sessions expire** on a 12h idle and 30d absolute timeout, and are revoked on
  logout and when the owning user is deleted.

The last administrator can't be deleted (that would lock everyone out), and an
admin can't delete their own account.

## Security note

Authentication is in place, but two things are still on the operator:

- **TLS.** Over plain HTTP the session cookie travels in the clear. Run behind a
  reverse proxy or a self-signed cert and set `FERROUS_COOKIE_SECURE=1`.
- **Real system access.** Any subsystem switched to a real backend acts with the
  daemon's privileges; see the pool safety table above before arming that one.
