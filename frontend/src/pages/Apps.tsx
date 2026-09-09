import { useMemo, useState } from "react";
import { api, fmtBytes } from "../lib/api";
import { useAsync } from "../lib/hooks";
import { Async, Badge, toast } from "../components/ui";

export default function Apps() {
  const apps = useAsync(api.apps, 5000);
  const catalog = useAsync(api.catalog);
  const [cat, setCat] = useState("All");

  const installedIds = useMemo(() => new Set(apps.data?.map((a) => a.catalog_id) ?? []), [apps.data]);
  const categories = useMemo(
    () => ["All", ...Array.from(new Set(catalog.data?.map((c) => c.category) ?? []))],
    [catalog.data]
  );

  const act = async (fn: () => Promise<unknown>, ok: string) => {
    try { await fn(); toast(ok); apps.reload(); }
    catch (e) { toast(String((e as Error).message), true); }
  };

  return (
    <>
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
                      <a className="btn sm" href={a.web_ui} target="_blank" rel="noreferrer">Open ↗</a>
                    )}
                    <button
                      className="btn sm ghost danger"
                      onClick={() => confirm(`Uninstall ${a.name}?`) && act(() => api.uninstallApp(a.id), `Uninstalled ${a.name}`)}
                    >
                      Uninstall
                    </button>
                  </div>
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
                    disabled={installed}
                    onClick={() => act(() => api.installApp({ catalog_id: c.id }), `Installed ${c.name}`)}
                  >
                    {installed ? "✓ Installed" : "Install"}
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
