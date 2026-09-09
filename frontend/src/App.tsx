import { NavLink, Navigate, Route, Routes, useLocation } from "react-router-dom";
import { api } from "./lib/api";
import { useAsync } from "./lib/hooks";
import { Toasts } from "./components/ui";
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

export default function App() {
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
