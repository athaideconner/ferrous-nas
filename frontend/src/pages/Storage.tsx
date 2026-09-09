import { useState } from "react";
import { api, fmtBytes, type RaidLevel } from "../lib/api";
import { useAsync } from "../lib/hooks";
import { Async, Badge, Meter, Modal, toast } from "../components/ui";

const smartTone: Record<string, string> = { passed: "green", warning: "yellow", failing: "red" };
const poolTone: Record<string, string> = { online: "green", degraded: "yellow", offline: "red", scrubbing: "blue" };
const kindLabel: Record<string, string> = { hdd: "HDD", ssd: "SSD", nvme: "NVMe" };

export default function Storage() {
  const disks = useAsync(api.disks);
  const pools = useAsync(api.pools);
  const datasets = useAsync(api.datasets);
  const [showPool, setShowPool] = useState(false);
  const [dsPool, setDsPool] = useState<string | null>(null);

  const reloadAll = () => {
    disks.reload();
    pools.reload();
    datasets.reload();
  };

  const poolName = (id: string) => pools.data?.find((p) => p.id === id)?.name ?? id;

  return (
    <>
      {/* ---- pools ---- */}
      <div className="row" style={{ justifyContent: "space-between", marginBottom: 12 }}>
        <div className="section-title" style={{ margin: 0 }}>Storage pools</div>
        <button className="btn primary sm" onClick={() => setShowPool(true)}>+ Create pool</button>
      </div>
      <Async state={pools}>
        {(ps) => (
          <div className="grid cols-2">
            {ps.map((p) => {
              const pct = (p.used_bytes / p.size_bytes) * 100;
              return (
                <div key={p.id} className="card">
                  <div className="card-head">
                    <div className="vstack">
                      <h2>{p.name}</h2>
                      <span className="faint">{p.raid_level} · {p.disk_ids.length} disks</span>
                    </div>
                    <Badge tone={poolTone[p.status]}>{p.status}</Badge>
                  </div>
                  <div className="row" style={{ justifyContent: "space-between", marginBottom: 6 }}>
                    <span className="faint">{fmtBytes(p.used_bytes)} used</span>
                    <span className="faint">{fmtBytes(p.size_bytes)} total</span>
                  </div>
                  <Meter pct={pct} />
                  <div className="row" style={{ marginTop: 14, gap: 8 }}>
                    <button
                      className="btn sm"
                      disabled={p.status === "scrubbing"}
                      onClick={async () => {
                        try { await api.scrubPool(p.id); toast(`Started scrub on ${p.name}`); reloadAll(); }
                        catch (e) { toast(String((e as Error).message), true); }
                      }}
                    >
                      🧹 Scrub
                    </button>
                    <button
                      className="btn sm danger"
                      onClick={async () => {
                        if (!confirm(`Destroy pool "${p.name}"? (mock)`)) return;
                        try { await api.deletePool(p.id); toast(`Destroyed ${p.name}`); reloadAll(); }
                        catch (e) { toast(String((e as Error).message), true); }
                      }}
                    >
                      Destroy
                    </button>
                  </div>
                </div>
              );
            })}
          </div>
        )}
      </Async>

      {/* ---- physical disks ---- */}
      <div className="section-title">Physical disks</div>
      <div className="card" style={{ padding: 0 }}>
        <Async state={disks}>
          {(ds) => (
            <table>
              <thead>
                <tr>
                  <th>Device</th><th>Model</th><th>Type</th><th>Capacity</th>
                  <th>Temp</th><th>S.M.A.R.T.</th><th>Pool</th>
                </tr>
              </thead>
              <tbody>
                {ds.map((d) => (
                  <tr key={d.id}>
                    <td className="mono">/dev/{d.device}</td>
                    <td>{d.model}</td>
                    <td><Badge tone="gray">{kindLabel[d.kind]}</Badge></td>
                    <td>{fmtBytes(d.size_bytes)}</td>
                    <td>{d.temp_c.toFixed(0)}°C</td>
                    <td><Badge tone={smartTone[d.smart]}>{d.smart}</Badge></td>
                    <td className="faint">{d.pool_id ? poolName(d.pool_id) : "—"}</td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Async>
      </div>

      {/* ---- datasets ---- */}
      <div className="row" style={{ justifyContent: "space-between", margin: "24px 0 12px" }}>
        <div className="section-title" style={{ margin: 0 }}>Datasets</div>
        <button className="btn sm" onClick={() => setDsPool(pools.data?.[0]?.id ?? null)}>+ New dataset</button>
      </div>
      <div className="card" style={{ padding: 0 }}>
        <Async state={datasets}>
          {(ds) => (
            <table>
              <thead>
                <tr><th>Name</th><th>Pool</th><th>Path</th><th>Used</th><th>Quota</th><th>Compression</th><th></th></tr>
              </thead>
              <tbody>
                {ds.map((d) => (
                  <tr key={d.id}>
                    <td><b>{d.name}</b></td>
                    <td className="faint">{poolName(d.pool_id)}</td>
                    <td className="mono faint">{d.path}</td>
                    <td>{fmtBytes(d.used_bytes)}</td>
                    <td className="faint">{d.quota_bytes ? fmtBytes(d.quota_bytes) : "none"}</td>
                    <td>{d.compression ? <Badge tone="accent">lz4</Badge> : <span className="faint">off</span>}</td>
                    <td className="right">
                      <button
                        className="btn sm ghost danger"
                        onClick={async () => {
                          if (!confirm(`Delete dataset "${d.name}"?`)) return;
                          try { await api.deleteDataset(d.id); toast(`Deleted ${d.name}`); reloadAll(); }
                          catch (e) { toast(String((e as Error).message), true); }
                        }}
                      >
                        Delete
                      </button>
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Async>
      </div>

      {showPool && (
        <CreatePoolModal
          freeDisks={disks.data?.filter((d) => !d.pool_id) ?? []}
          onClose={() => setShowPool(false)}
          onDone={() => { setShowPool(false); reloadAll(); }}
        />
      )}
      {dsPool !== null && (
        <CreateDatasetModal
          pools={pools.data ?? []}
          initialPool={dsPool}
          onClose={() => setDsPool(null)}
          onDone={() => { setDsPool(null); reloadAll(); }}
        />
      )}
    </>
  );
}

function CreatePoolModal({
  freeDisks, onClose, onDone,
}: {
  freeDisks: { id: string; device: string; model: string; size_bytes: number }[];
  onClose: () => void;
  onDone: () => void;
}) {
  const [name, setName] = useState("");
  const [raid, setRaid] = useState<RaidLevel>("mirror");
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);

  const toggle = (id: string) => {
    const next = new Set(selected);
    next.has(id) ? next.delete(id) : next.add(id);
    setSelected(next);
  };

  const submit = async () => {
    setBusy(true);
    try {
      await api.createPool({ name, raid_level: raid, disk_ids: [...selected] });
      toast(`Created pool ${name}`);
      onDone();
    } catch (e) {
      toast(String((e as Error).message), true);
      setBusy(false);
    }
  };

  return (
    <Modal title="Create storage pool" onClose={onClose}>
      <label className="field">
        <span>Pool name</span>
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. vault" />
      </label>
      <label className="field">
        <span>RAID level</span>
        <select value={raid} onChange={(e) => setRaid(e.target.value as RaidLevel)}>
          <option value="stripe">Stripe (no redundancy)</option>
          <option value="mirror">Mirror (raid1)</option>
          <option value="raidz1">RAIDZ1 (raid5)</option>
          <option value="raidz2">RAIDZ2 (raid6)</option>
        </select>
      </label>
      <label className="field">
        <span>Member disks ({selected.size} selected)</span>
      </label>
      {freeDisks.length === 0 && <div className="faint">No free disks available.</div>}
      <div className="vstack" style={{ gap: 6, maxHeight: 180, overflow: "auto" }}>
        {freeDisks.map((d) => (
          <label key={d.id} className="checkbox">
            <input type="checkbox" checked={selected.has(d.id)} onChange={() => toggle(d.id)} />
            <span className="mono">/dev/{d.device}</span>
            <span className="faint">{d.model} · {fmtBytes(d.size_bytes)}</span>
          </label>
        ))}
      </div>
      <div className="actions">
        <button className="btn ghost" onClick={onClose}>Cancel</button>
        <button className="btn primary" disabled={busy || !name || selected.size === 0} onClick={submit}>
          Create
        </button>
      </div>
    </Modal>
  );
}

function CreateDatasetModal({
  pools, initialPool, onClose, onDone,
}: {
  pools: { id: string; name: string }[];
  initialPool: string;
  onClose: () => void;
  onDone: () => void;
}) {
  const [pool, setPool] = useState(initialPool);
  const [name, setName] = useState("");
  const [quota, setQuota] = useState("");
  const [compression, setCompression] = useState(true);
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    setBusy(true);
    try {
      await api.createDataset({
        pool_id: pool,
        name,
        quota_gb: quota ? Number(quota) : undefined,
        compression,
      });
      toast(`Created dataset ${name}`);
      onDone();
    } catch (e) {
      toast(String((e as Error).message), true);
      setBusy(false);
    }
  };

  return (
    <Modal title="Create dataset" onClose={onClose}>
      <label className="field">
        <span>Pool</span>
        <select value={pool} onChange={(e) => setPool(e.target.value)}>
          {pools.map((p) => <option key={p.id} value={p.id}>{p.name}</option>)}
        </select>
      </label>
      <label className="field">
        <span>Name</span>
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. projects" />
      </label>
      <label className="field">
        <span>Quota (GB, optional)</span>
        <input value={quota} onChange={(e) => setQuota(e.target.value.replace(/\D/g, ""))} placeholder="unlimited" />
      </label>
      <label className="checkbox" style={{ marginBottom: 4 }}>
        <input type="checkbox" checked={compression} onChange={(e) => setCompression(e.target.checked)} />
        Enable lz4 compression
      </label>
      <div className="actions">
        <button className="btn ghost" onClick={onClose}>Cancel</button>
        <button className="btn primary" disabled={busy || !name} onClick={submit}>Create</button>
      </div>
    </Modal>
  );
}
