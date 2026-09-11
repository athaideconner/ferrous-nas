import { useCallback, useEffect, useState } from "react";
import { NavLink, Navigate, Route, Routes, useLocation } from "react-router-dom";
import { api, UNAUTHORIZED_EVENT, type User } from "./lib/api";
import { useAsync } from "./lib/hooks";
import { Spinner, Toasts } from "./components/ui";
import Auth from "./pages/Auth";
import Dashboard from "./pages/Dashboard";
import Storage from "./pages/Storage";
import Shares from "./pages/Shares";
import Apps from "./pages/Apps";
import Users from "./pages/Users";
import Network from "./pages/Network";
import System from "./pages/System";

const NAV = [
  { to: "/dashboard", ico: "📊", label: "Dashboard" },
  { to: "/storage", ico: "💽", label: "Storage" },
  { to: "/shares", ico: "📁", label: "Shares" },
  { to: "/apps", ico: "🧩", label: "Apps" },
  { to: "/users", ico: "👥", label: "Users" },
  { to: "/network", ico: "🌐", label: "Network" },
  { to: "/system", ico: "⚙️", label: "System" },
];

const TITLES: Record<string, string> = {
  "/dashboard": "Dashboard",
  "/storage": "Storage",
  "/shares": "Shares",
  "/apps": "Apps",
  "/users": "Users & Groups",
  "/network": "Network",
  "/system": "System",
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
  const title = TITLES[loc.pathname] ?? "FerrousNAS";
  const tel = useAsync(api.telemetrySource);
  const live = tel.data?.source === "linux";

  return (
    <div className="app">
      <aside className="sidebar">
        <div className="brand">
          <div className="logo">🗄️</div>
          <div>
            <div className="name">FerrousNAS</div>
            <div className="ver">control plane · mock</div>
          </div>
        </div>
        <nav className="nav">
          {NAV.map((n) => (
            <NavLink key={n.to} to={n.to} className={({ isActive }) => (isActive ? "active" : "")}>
              <span className="ico">{n.ico}</span>
              {n.label}
            </NavLink>
          ))}
        </nav>
        <div className="foot">
          All operations are simulated.
          <br />
          No real disks are touched.
        </div>
      </aside>

      <main className="main">
        <header className="topbar">
          <h1>{title}</h1>
          <div className="spacer" />
          <span className="pill" title={live ? "Reading real host telemetry (read-only)" : "Telemetry is simulated"}>
            <span className="dot" style={{ background: live ? "var(--green)" : "var(--yellow)", boxShadow: `0 0 8px ${live ? "var(--green)" : "var(--yellow)"}` }} />
            telemetry: {tel.data?.source ?? "…"}
          </span>
          <span className="pill">
            <span className="dot" /> System healthy
          </span>
          {user && (
            <span className="pill" title={user.is_admin ? "Administrator" : "Standard user"}>
              {user.is_admin ? "🛡️" : "👤"} {user.username}
            </span>
          )}
          <button
            className="btn sm ghost"
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
        </header>
        <div className="content">
          <Routes>
            <Route path="/" element={<Navigate to="/dashboard" replace />} />
            <Route path="/dashboard" element={<Dashboard />} />
            <Route path="/storage" element={<Storage />} />
            <Route path="/shares" element={<Shares />} />
            <Route path="/apps" element={<Apps />} />
            <Route path="/users" element={<Users />} />
            <Route path="/network" element={<Network />} />
            <Route path="/system" element={<System />} />
            <Route path="*" element={<div className="empty">Page not found</div>} />
          </Routes>
        </div>
      </main>
      <Toasts />
    </div>
  );
}
