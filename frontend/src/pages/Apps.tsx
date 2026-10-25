import { useMemo, useState } from "react";
import { api, fmtBytes } from "../lib/api";
import { useAsync } from "../lib/hooks";
import { Async, Badge, toast } from "../components/ui";

export default function Apps() {
  const apps = useAsync(api.apps, 5000);
  const catalog = useAsync(api.catalog);
  const files = useAsync(api.filesLocation);
  const [moving, setMoving] = useState(false);
  const [cat, setCat] = useState("All");
  const [installing, setInstalling] = useState<Set<string>>(new Set());

  const installedIds = useMemo(() => new Set(apps.data?.map((a) => a.catalog_id) ?? []), [apps.data]);
  const categories = useMemo(
    () => ["All", ...Array.from(new Set(catalog.data?.map((c) => c.category) ?? []))],
    [catalog.data]
  );

  const act = async (fn: () => Promise<unknown>, ok: string) => {
    try { await fn(); toast(ok); apps.reload(); }
    catch (e) { toast(String((e as Error).message), true); }
  };

  const moveFiles = async (path: string) => {
    const label = files.data?.options.find((o) => o.path === path)?.label ?? path;
    if (!confirm(
      `Keep app media & files on ${label}?\n\nInstalled apps that use them restart with the new location. ` +
      `Files already in the old location are not moved.`
    )) return;
    setMoving(true);
    await act(() => api.setFilesLocation(path), `Apps now keep their files on ${label}`);
    files.reload();
    setMoving(false);
  };

  return (
    <>
      <div className="card" style={{ marginBottom: 22 }}>
        <div className="row" style={{ justifyContent: "space-between", gap: 16, flexWrap: "wrap" }}>
          <div className="vstack" style={{ gap: 4, maxWidth: 640 }}>
            <b>Media &amp; files</b>
            <span className="faint" style={{ fontSize: 13 }}>
              Where Jellyfin and Plex find media, qBittorrent saves downloads, Immich keeps photos and
              Paperless picks up scans — in <span className="mono">media</span>,{" "}
              <span className="mono">media/downloads</span>, <span className="mono">photos</span> and{" "}
              <span className="mono">scans</span>. Pick a pool dataset, and share it to add files from your computer.
            </span>
          </div>
          <Async state={files}>
            {(f) => (
              <select value={f.files_root} disabled={moving} onChange={(e) => moveFiles(e.target.value)}>
                {f.options.map((o) => (
                  <option key={o.path} value={o.path}>
                    {o.label} ({o.path})
                  </option>
                ))}
              </select>
            )}
          </Async>
        </div>
        {moving && <div className="faint" style={{ marginTop: 8 }}>Moving apps to the new location…</div>}
      </div>

      <div className="section-title" style={{ marginTop: 0 }}>Installed</div>
      <Async state={apps}>
        {(list) =>
          list.length === 0 ? (
            <div className="empty">Nothing installed yet — pick something from the store below.</div>
          ) : (
            <div className="grid cols-3">
              {list.map((a) => (
                <div key={a.id} className="appcard">
                  <div className="row" style={{ justifyContent: "space-between" }}>
                    <span className="ico">{a.icon}</span>
                    <Badge tone={a.state === "running" ? "green" : a.state === "error" ? "red" : "gray"}>{a.state}</Badge>
                  </div>
                  <div>
                    <div className="title">{a.name}</div>
                    <div className="tag mono">{a.image}</div>
                  </div>
                  <div className="row faint" style={{ gap: 14, fontSize: 12 }}>
                    <span>CPU {a.cpu_percent.toFixed(1)}%</span>
                    <span>RAM {fmtBytes(a.mem_bytes)}</span>
                    <span>:{a.host_port}</span>
                  </div>
                  <div className="row" style={{ gap: 8, flexWrap: "wrap" }}>
                    {a.state === "running" ? (
                      <button className="btn sm" onClick={() => act(() => api.stopApp(a.id), `Stopped ${a.name}`)}>⏸ Stop</button>
                    ) : (
                      <button className="btn sm" onClick={() => act(() => api.startApp(a.id), `Started ${a.name}`)}>▶ Start</button>
                    )}
                    {a.web_ui && a.state === "running" && (
                      // The app is on the same machine as the dashboard, so
                      // the address the browser used for this page reaches it.
                      <a className="btn sm" href={a.web_ui.replace("{host}", window.location.hostname)} target="_blank" rel="noreferrer">Open ↗</a>
                    )}
                    <button
                      className="btn sm ghost danger"
                      onClick={() =>
                        confirm(`Uninstall ${a.name}? Its data is kept, and comes back if you install it again.`) &&
                        act(() => api.uninstallApp(a.id), `Uninstalled ${a.name}`)
                      }
                    >
                      Uninstall
                    </button>
                  </div>
                  {a.hint && <div className="faint" style={{ fontSize: 12 }}>🔑 {a.hint}</div>}
                </div>
              ))}
            </div>
          )
        }
      </Async>

      <div className="row" style={{ justifyContent: "space-between", alignItems: "center", margin: "26px 0 12px" }}>
        <div className="section-title" style={{ margin: 0 }}>App store</div>
        <div className="row" style={{ gap: 6, flexWrap: "wrap" }}>
          {categories.map((c) => (
            <button key={c} className={"btn sm" + (c === cat ? " primary" : " ghost")} onClick={() => setCat(c)}>{c}</button>
          ))}
        </div>
      </div>
      <Async state={catalog}>
        {(list) => (
          <div className="grid cols-3">
            {list.filter((c) => cat === "All" || c.category === cat).map((c) => {
              const installed = installedIds.has(c.id);
              const busy = installing.has(c.id);
              return (
                <div key={c.id} className="appcard">
                  <div className="row" style={{ justifyContent: "space-between" }}>
                    <span className="ico">{c.icon}</span>
                    <Badge tone="gray">{c.category}</Badge>
                  </div>
                  <div>
                    <div className="title">{c.name}</div>
                    <div className="tag">{c.tagline}</div>
                  </div>
                  <div className="desc">{c.description}</div>
                  <button
                    className={"btn sm" + (installed ? "" : " primary")}
                    disabled={installed || busy}
                    onClick={async () => {
                      // Downloading an app can take minutes.
                      setInstalling((s) => new Set(s).add(c.id));
                      await act(() => api.installApp({ catalog_id: c.id }), `Installed ${c.name}`);
                      setInstalling((s) => { const n = new Set(s); n.delete(c.id); return n; });
                    }}
                  >
                    {installed ? "✓ Installed" : busy ? "Downloading and installing…" : "Install"}
                  </button>
                </div>
              );
            })}
          </div>
        )}
      </Async>
    </>
  );
}
