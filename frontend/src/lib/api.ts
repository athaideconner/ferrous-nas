// Typed client for the FerrousNAS daemon. Field names and enum values mirror
// the Rust models exactly (serde is configured to emit these strings).

const BASE = "/api/v1";

export type DiskKind = "hdd" | "ssd" | "nvme";
export type SmartStatus = "passed" | "warning" | "failing";
export type RaidLevel = "stripe" | "mirror" | "raidz1" | "raidz2";
export type PoolStatus = "online" | "degraded" | "offline" | "scrubbing";
export type ShareKind = "smb" | "nfs";
export type AppLifecycle = "running" | "stopped" | "installing" | "error";
export type AlertLevel = "info" | "warning" | "critical";

export interface SystemInfo {
  hostname: string;
  product: string;
  version: string;
  kernel: string;
  uptime_secs: number;
  cpu: {
    model: string;
    cores: number;
    threads: number;
    usage_percent: number;
    temp_c: number;
  };
  memory: {
    total_bytes: number;
    used_bytes: number;
    swap_total_bytes: number;
    swap_used_bytes: number;
  };
  load_avg: [number, number, number];
}

export interface StatPoint {
  t: number;
  cpu_percent: number;
  mem_percent: number;
  net_rx_mbps: number;
  net_tx_mbps: number;
  disk_read_mbps: number;
  disk_write_mbps: number;
}

export interface Disk {
  id: string;
  device: string;
  model: string;
  serial: string;
  size_bytes: number;
  kind: DiskKind;
  temp_c: number;
  smart: SmartStatus;
  power_on_hours: number;
  pool_id: string | null;
}

export interface Pool {
  id: string;
  name: string;
  raid_level: RaidLevel;
  status: PoolStatus;
  size_bytes: number;
  used_bytes: number;
  disk_ids: string[];
  scrub_progress: number | null;
}

export interface Dataset {
  id: string;
  pool_id: string;
  name: string;
  path: string;
  used_bytes: number;
  quota_bytes: number | null;
  compression: boolean;
}

export interface Share {
  id: string;
  name: string;
  kind: ShareKind;
  dataset_id: string;
  path: string;
  enabled: boolean;
  read_only: boolean;
  guest_ok: boolean;
  allowed_users: string[];
}

export interface CatalogApp {
  id: string;
  name: string;
  tagline: string;
  description: string;
  icon: string;
  category: string;
  image: string;
  default_port: number;
}

export interface InstalledApp {
  id: string;
  catalog_id: string;
  name: string;
  icon: string;
  category: string;
  image: string;
  state: AppLifecycle;
  host_port: number;
  cpu_percent: number;
  mem_bytes: number;
  web_ui: string | null;
  created_at: string;
}

export interface User {
  id: string;
  username: string;
  full_name: string;
  is_admin: boolean;
  groups: string[];
  created_at: string;
}

export interface Group {
  id: string;
  name: string;
  members: string[];
}

export interface NetInterface {
  name: string;
  mac: string;
  ipv4: string | null;
  ipv6: string | null;
  kind: string;
  up: boolean;
  speed_mbps: number;
  rx_bytes: number;
  tx_bytes: number;
}

export interface Alert {
  id: string;
  level: AlertLevel;
  title: string;
  message: string;
  created_at: string;
  acknowledged: boolean;
}

// --- core fetch helper ----------------------------------------------------

async function req<T>(path: string, init?: RequestInit): Promise<T> {
  const res = await fetch(BASE + path, {
    headers: { "Content-Type": "application/json" },
    ...init,
  });
  if (!res.ok) {
    let detail = res.statusText;
    try {
      const body = await res.json();
      if (body?.error) detail = body.error;
    } catch {
      /* ignore non-JSON error bodies */
    }
    throw new Error(detail);
  }
  // 204-less API: every endpoint returns JSON.
  return (await res.json()) as T;
}

const post = <T>(path: string, body?: unknown) =>
  req<T>(path, { method: "POST", body: body ? JSON.stringify(body) : undefined });
const patch = <T>(path: string, body: unknown) =>
  req<T>(path, { method: "PATCH", body: JSON.stringify(body) });
const del = <T>(path: string) => req<T>(path, { method: "DELETE" });

export const api = {
  system: () => req<SystemInfo>("/system"),
  stats: (points = 60) => req<StatPoint[]>(`/system/stats?points=${points}`),
  reboot: () => post<{ ok: boolean }>("/system/reboot"),
  shutdown: () => post<{ ok: boolean }>("/system/shutdown"),

  alerts: () => req<Alert[]>("/alerts"),
  ackAlert: (id: string) => post<Alert>(`/alerts/${id}/ack`),

  disks: () => req<Disk[]>("/storage/disks"),
  pools: () => req<Pool[]>("/storage/pools"),
  createPool: (body: { name: string; raid_level: RaidLevel; disk_ids: string[] }) =>
    post<Pool>("/storage/pools", body),
  deletePool: (id: string) => del<{ ok: boolean }>(`/storage/pools/${id}`),
  scrubPool: (id: string) => post<Pool>(`/storage/pools/${id}/scrub`),
  datasets: () => req<Dataset[]>("/storage/datasets"),
  createDataset: (body: {
    pool_id: string;
    name: string;
    quota_gb?: number;
    compression?: boolean;
  }) => post<Dataset>("/storage/datasets", body),
  deleteDataset: (id: string) => del<{ ok: boolean }>(`/storage/datasets/${id}`),

  shares: () => req<Share[]>("/shares"),
  createShare: (body: {
    name: string;
    kind: ShareKind;
    dataset_id: string;
    read_only?: boolean;
    guest_ok?: boolean;
    allowed_users?: string[];
  }) => post<Share>("/shares", body),
  patchShare: (id: string, body: Partial<Pick<Share, "enabled" | "read_only" | "guest_ok">>) =>
    patch<Share>(`/shares/${id}`, body),
  deleteShare: (id: string) => del<{ ok: boolean }>(`/shares/${id}`),

  catalog: () => req<CatalogApp[]>("/apps/catalog"),
  apps: () => req<InstalledApp[]>("/apps"),
  installApp: (body: { catalog_id: string; host_port?: number }) =>
    post<InstalledApp>("/apps", body),
  startApp: (id: string) => post<InstalledApp>(`/apps/${id}/start`),
  stopApp: (id: string) => post<InstalledApp>(`/apps/${id}/stop`),
  uninstallApp: (id: string) => del<{ ok: boolean }>(`/apps/${id}`),

  users: () => req<User[]>("/users"),
  createUser: (body: {
    username: string;
    full_name: string;
    is_admin?: boolean;
    groups?: string[];
  }) => post<User>("/users", body),
  deleteUser: (id: string) => del<{ ok: boolean }>(`/users/${id}`),
  groups: () => req<Group[]>("/groups"),

  interfaces: () => req<NetInterface[]>("/network/interfaces"),
};

// --- formatting helpers ---------------------------------------------------

export function fmtBytes(n: number): string {
  if (n <= 0) return "0 B";
  const units = ["B", "KB", "MB", "GB", "TB", "PB"];
  const i = Math.min(units.length - 1, Math.floor(Math.log10(n) / 3));
  const v = n / Math.pow(1000, i);
  return `${v.toFixed(v >= 100 || i === 0 ? 0 : 1)} ${units[i]}`;
}

export function fmtUptime(secs: number): string {
  const d = Math.floor(secs / 86400);
  const h = Math.floor((secs % 86400) / 3600);
  const m = Math.floor((secs % 3600) / 60);
  return `${d}d ${h}h ${m}m`;
}
