import { useCallback, useEffect, useState, type ReactNode } from "react";
import { Navigate, Route, Routes, useLocation, useNavigate } from "react-router-dom";
import { api, UNAUTHORIZED_EVENT, type User } from "./lib/api";
import { useAsync } from "./lib/hooks";
import { Spinner, Toasts } from "./components/ui";
import Auth from "./pages/Auth";
import Desktop, { SYSTEM_APPS } from "./pages/Desktop";
import Dashboard from "./pages/Dashboard";
import Storage from "./pages/Storage";
import Shares from "./pages/Shares";
import Apps from "./pages/Apps";
import Users from "./pages/Users";
import Network from "./pages/Network";
import System from "./pages/System";

/** Window titles for the pages that open over the desktop. */
const TITLES: Record<string, string> = {
  "/dashboard": "Monitor",
  "/storage": "Storage",
  "/shares": "Shares",
  "/apps": "App Store",
  "/users": "Users & Groups",
  "/network": "Network",
  "/system": "Settings",
};

type Phase = "loading" | "setup" | "login" | "ready";

export default function App() {
  const [phase, setPhase] = useState<Phase>("loading");
  const [user, setUser] = useState<User | null>(null);

  const check = useCallback(async () => {
    try {
      const status = await api.auth.status();
      if (status.setup_required) {
        setPhase("setup");
        return;
      }
      // With auth disabled this still succeeds, returning the local admin.
      const me = await api.auth.me();
      setUser(me);
      setPhase("ready");
    } catch {
      setPhase("login");
    }
  }, []);

  useEffect(() => {
    check();
  }, [check]);

  // Any 401 from anywhere (e.g. an expired session) drops us back to sign-in.
  useEffect(() => {
    const onUnauthorized = () => setPhase("login");
    window.addEventListener(UNAUTHORIZED_EVENT, onUnauthorized);
    return () => window.removeEventListener(UNAUTHORIZED_EVENT, onUnauthorized);
  }, []);

  if (phase === "loading") return <Spinner />;
  if (phase === "setup" || phase === "login") {
    return <Auth mode={phase} onDone={check} />;
  }
  return <Shell user={user} onSignedOut={() => { setUser(null); setPhase("login"); }} />;
}

function Shell({ user, onSignedOut }: { user: User | null; onSignedOut: () => void }) {
  const loc = useLocation();
  const navigate = useNavigate();
  const windowPath = TITLES[loc.pathname] ? loc.pathname : null;

  // Esc closes the open window, back to the desktop.
  useEffect(() => {
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape" && windowPath && !document.querySelector(".modal-backdrop")) navigate("/");
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [windowPath, navigate]);

  return (
    <div className="desktop">
      <TopBar user={user} onSignedOut={onSignedOut} />
      <Desktop user={user} />
      {windowPath && (
        <AppWindow path={windowPath} onClose={() => navigate("/")}>
          <Routes>
            <Route path="/dashboard" element={<Dashboard />} />
            <Route path="/storage" element={<Storage />} />
            <Route path="/shares" element={<Shares />} />
            <Route path="/apps" element={<Apps />} />
            <Route path="/users" element={<Users />} />
            <Route path="/network" element={<Network />} />
            <Route path="/system" element={<System />} />
          </Routes>
        </AppWindow>
      )}
      {!windowPath && loc.pathname !== "/" && <Navigate to="/" replace />}
      <Toasts />
    </div>
  );
}

/** A page opened from the desktop: a large light window over the wallpaper. */
function AppWindow({ path, onClose, children }: { path: string; onClose: () => void; children: ReactNode }) {
  const app = SYSTEM_APPS.find((a) => a.to === path);
  return (
    <div className="window-backdrop" onClick={onClose}>
      <section className="window" role="dialog" aria-label={TITLES[path]} onClick={(e) => e.stopPropagation()}>
        <header className="window-bar">
          {app && (
            <span className="window-icon" style={{ background: `linear-gradient(145deg, ${app.tint}, ${app.tint}99)` }}>
              {app.icon}
            </span>
          )}
          <h1>{TITLES[path]}</h1>
          <button className="window-close" aria-label="Close" onClick={onClose}>✕</button>
        </header>
        <div className="window-body">{children}</div>
      </section>
    </div>
  );
}

function TopBar({ user, onSignedOut }: { user: User | null; onSignedOut: () => void }) {
  const navigate = useNavigate();
  const sys = useAsync(api.system, 60000);
  const alerts = useAsync(api.alerts, 15000);
  const [open, setOpen] = useState<"alerts" | "user" | null>(null);
  const unacked = alerts.data?.filter((a) => !a.acknowledged) ?? [];
  const toggle = (m: "alerts" | "user") => setOpen(open === m ? null : m);

  return (
    <header className="topbar">
      <div className="brand">
        <div className="logo">🗄️</div>
        <div>
          <div className="name">FerrousNAS</div>
          <div className="ver">{sys.data?.hostname ?? ""}</div>
        </div>
      </div>
      <div className="spacer" />
      <button className="glass round" title="App Store" onClick={() => navigate("/apps")}>🛍️</button>
      <div className="pop-anchor">
        <button className="glass round" title="Notifications" onClick={() => toggle("alerts")}>
          🔔{unacked.length > 0 && <span className="count">{unacked.length}</span>}
        </button>
        {open === "alerts" && (
          <div className="popover glass">
            <div className="pop-title">Notifications</div>
            {alerts.data?.length ? (
              alerts.data.slice(0, 8).map((a) => (
                <div key={a.id} className={"pop-item" + (a.acknowledged ? " done" : "")}>
                  <span className={"lvl " + a.level} />
                  <div>
                    <div>{a.title}</div>
                    <div className="pop-sub">{a.message}</div>
                  </div>
                  {!a.acknowledged && (
                    <button
                      className="pop-ack"
                      onClick={async () => { await api.ackAlert(a.id).catch(() => {}); alerts.reload(); }}
                    >
                      ✓
                    </button>
                  )}
                </div>
              ))
            ) : (
              <div className="pop-sub">Nothing new.</div>
            )}
          </div>
        )}
      </div>
      <button className="glass round" title="Settings" onClick={() => navigate("/system")}>⚙️</button>
      <div className="pop-anchor">
        <button className="glass avatar" title={user?.username} onClick={() => toggle("user")}>
          {(user?.full_name || user?.username || "?").slice(0, 1).toUpperCase()}
        </button>
        {open === "user" && (
          <div className="popover glass">
            <div className="pop-title">{user?.full_name || user?.username}</div>
            <div className="pop-sub">{user?.is_admin ? "Administrator" : "Standard user"} · {user?.username}</div>
            <SourcesNote />
            <button
              className="pop-btn"
              onClick={async () => {
                try {
                  await api.auth.logout();
                } finally {
                  onSignedOut();
                }
              }}
            >
              Sign out
            </button>
          </div>
        )}
      </div>
    </header>
  );
}

/** Which subsystems change this machine for real, and which are simulated. */
function SourcesNote() {
  const tel = useAsync(api.telemetrySource);
  if (!tel.data) return null;
  const subsystems = [
    ["apps", tel.data.apps],
    ["storage", tel.data.pools],
    ["shares", tel.data.shares],
    ["users", tel.data.users],
    ["power", tel.data.power],
    ["network", tel.data.source],
  ] as const;
  const simulated = subsystems.filter(([, src]) => !src || src === "mock").map(([name]) => name);
  const real = subsystems.map(([name]) => name).filter((name) => !simulated.includes(name));
  return (
    <div className="pop-note">
      {simulated.length === 0
        ? "Everything here changes this machine for real (notifications are sample data)."
        : simulated.length === subsystems.length
          ? "Demo mode: everything is simulated; no real disks are touched."
          : `Real: ${real.join(", ")}. Simulated: ${simulated.join(", ")}.`}
    </div>
  );
}
