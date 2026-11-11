import { useEffect, useState } from "react";
import { useNavigate } from "react-router-dom";
import { api, fmtBytes, fmtUptime, type InstalledApp, type User } from "../lib/api";
import { useAsync } from "../lib/hooks";
import { toast } from "../components/ui";

/** The built-in "apps" on the desktop: each opens a window over it. */
export const SYSTEM_APPS = [
  { to: "/apps", icon: "🛍️", label: "App Store", tint: "#ff7a45" },
  { to: "/dashboard", icon: "📊", label: "Monitor", tint: "#5b8def" },
  { to: "/storage", icon: "💽", label: "Storage", tint: "#8a6cf0" },
  { to: "/shares", icon: "📁", label: "Shares", tint: "#f2b33d" },
  { to: "/users", icon: "👥", label: "Users", tint: "#3fbf8f" },
  { to: "/network", icon: "🌐", label: "Network", tint: "#3ab4e0" },
  { to: "/system", icon: "⚙️", label: "Settings", tint: "#8592a6" },
];

/**
 * The home screen, in the style of a NAS desktop (ZimaOS / CasaOS): widgets
 * on the left, the system's own tools and the installed apps as a grid of
 * icons on the right, all over the wallpaper.
 */
export default function Desktop({ user }: { user: User | null }) {
  return (
    <div className="desktop-body">
      <aside className="widgets">
        <ClockWidget user={user} />
        <SystemWidget />
        <StorageWidget />
        <NetworkWidget />
      </aside>
      <AppGrid />
    </div>
  );
}

function ClockWidget({ user }: { user: User | null }) {
  const [now, setNow] = useState(new Date());
  useEffect(() => {
    const id = setInterval(() => setNow(new Date()), 1000);
    return () => clearInterval(id);
  }, []);
  const h = now.getHours();
  const greeting = h < 5 ? "Good night" : h < 12 ? "Good morning" : h < 18 ? "Good afternoon" : "Good evening";
  return (
    <div className="glass widget clock">
      <div className="time">{now.toLocaleTimeString([], { hour: "2-digit", minute: "2-digit" })}</div>
      <div className="date">{now.toLocaleDateString([], { weekday: "long", month: "long", day: "numeric" })}</div>
      <div className="greet">
        {greeting}
        {user ? `, ${user.full_name || user.username}` : ""}
      </div>
    </div>
  );
}

function SystemWidget() {
  const sys = useAsync(api.system, 3000);
  const navigate = useNavigate();
  const s = sys.data;
  const mem = s ? (s.memory.used_bytes / s.memory.total_bytes) * 100 : 0;
  return (
    <button className="glass widget" onClick={() => navigate("/dashboard")}>
      <div className="widget-title">System status</div>
      {s ? (
        <>
          <div className="gauges">
            <Gauge pct={s.cpu.usage_percent} label="CPU" />
            <Gauge pct={mem} label="RAM" />
          </div>
          <div className="widget-foot">
            {s.cpu.temp_c > 0 && <span>🌡 {s.cpu.temp_c.toFixed(0)}°C</span>}
            <span>⏱ {fmtUptime(s.uptime_secs)}</span>
          </div>
        </>
      ) : (
        <div className="widget-foot">Loading…</div>
      )}
    </button>
  );
}

/** A small ring gauge for the widgets, drawn for light-on-glass. */
function Gauge({ pct, label }: { pct: number; label: string }) {
  const r = 30;
  const c = 2 * Math.PI * r;
  const p = Math.max(0, Math.min(100, pct || 0));
  const color = p > 85 ? "#ff6b5b" : p > 65 ? "#ffc145" : "#7cf0b2";
  return (
    <div className="gauge">
      <svg width="76" height="76" viewBox="0 0 76 76">
        <circle cx="38" cy="38" r={r} fill="none" stroke="rgba(255,255,255,0.18)" strokeWidth="7" />
        <circle
          cx="38" cy="38" r={r} fill="none" stroke={color} strokeWidth="7" strokeLinecap="round"
          strokeDasharray={`${(p / 100) * c} ${c}`} transform="rotate(-90 38 38)"
          style={{ transition: "stroke-dasharray 0.6s" }}
        />
        <text x="38" y="43" textAnchor="middle" fontSize="15" fontWeight="700" fill="#fff">{p.toFixed(0)}%</text>
      </svg>
      <span>{label}</span>
    </div>
  );
}

function StorageWidget() {
  const pools = useAsync(api.pools, 10000);
  const navigate = useNavigate();
  return (
    <button className="glass widget" onClick={() => navigate("/storage")}>
      <div className="widget-title">Storage</div>
      {pools.data && pools.data.length === 0 && <div className="widget-foot">No pools yet — create one in Storage.</div>}
      {pools.data?.map((p) => {
        const pct = p.size_bytes ? (p.used_bytes / p.size_bytes) * 100 : 0;
        return (
          <div key={p.id} className="pool-row">
            <div className="pool-line">
              <b>{p.name}</b>
              <span>{fmtBytes(p.used_bytes)} / {fmtBytes(p.size_bytes)}</span>
            </div>
            <div className="glass-meter"><span style={{ width: `${Math.max(2, pct)}%` }} /></div>
          </div>
        );
      })}
    </button>
  );
}

function NetworkWidget() {
  const stats = useAsync(() => api.stats(2), 3000);
  const ifaces = useAsync(api.interfaces, 15000);
  const navigate = useNavigate();
  const last = stats.data?.at(-1);
  const ip = ifaces.data?.find((n) => n.up && n.ipv4 && n.kind !== "bridge")?.ipv4?.split("/")[0];
  return (
    <button className="glass widget" onClick={() => navigate("/network")}>
      <div className="widget-title">Network</div>
      <div className="net-rates">
        <div><span className="arrow">↓</span>{last ? last.net_rx_mbps.toFixed(1) : "–"}<small> Mb/s</small></div>
        <div><span className="arrow">↑</span>{last ? last.net_tx_mbps.toFixed(1) : "–"}<small> Mb/s</small></div>
      </div>
      {ip && <div className="widget-foot mono">{ip}</div>}
    </button>
  );
}

function AppGrid() {
  const apps = useAsync(api.apps, 5000);
  const navigate = useNavigate();
  return (
    <main className="app-grid">
      {SYSTEM_APPS.map((a) => (
        <button key={a.to} className="tile" onClick={() => navigate(a.to)}>
          <span className="tile-icon" style={{ background: `linear-gradient(145deg, ${a.tint}, ${a.tint}99)` }}>{a.icon}</span>
          <span className="tile-name">{a.label}</span>
        </button>
      ))}
      {apps.data?.map((a) => <AppTile key={a.id} app={a} reload={apps.reload} />)}
    </main>
  );
}

function AppTile({ app, reload }: { app: InstalledApp; reload: () => void }) {
  const [menu, setMenu] = useState(false);
  const running = app.state === "running";
  const url = app.web_ui?.replace("{host}", window.location.hostname);

  const act = async (fn: () => Promise<unknown>, ok: string) => {
    setMenu(false);
    try { await fn(); toast(ok); reload(); }
    catch (e) { toast(String((e as Error).message), true); }
  };

  return (
    <div className="tile" onMouseLeave={() => setMenu(false)}>
      <button
        className="tile-open"
        title={running ? `Open ${app.name}` : `${app.name} is stopped`}
        onClick={() => (running && url ? window.open(url, "_blank", "noreferrer") : setMenu(true))}
      >
        <span className={"tile-icon app" + (running ? "" : " stopped")}>
          {app.icon}
          <span className={"state-dot " + app.state} />
        </span>
        <span className="tile-name">{app.name}</span>
      </button>
      <button className="tile-more" aria-label={`${app.name} options`} onClick={() => setMenu(!menu)}>⋯</button>
      {menu && (
        <div className="tile-menu glass">
          {running && url && <a href={url} target="_blank" rel="noreferrer" onClick={() => setMenu(false)}>Open</a>}
          {running ? (
            <button onClick={() => act(() => api.stopApp(app.id), `Stopped ${app.name}`)}>Stop</button>
          ) : (
            <button onClick={() => act(() => api.startApp(app.id), `Started ${app.name}`)}>Start</button>
          )}
          <button
            className="danger"
            onClick={() =>
              confirm(`Uninstall ${app.name}? Its data is kept, and comes back if you install it again.`) &&
              act(() => api.uninstallApp(app.id), `Uninstalled ${app.name}`)
            }
          >
            Uninstall
          </button>
          {app.hint && <div className="tile-hint">🔑 {app.hint}</div>}
        </div>
      )}
    </div>
  );
}
