import { useState } from "react";
import { api, type Dataset, type ShareKind } from "../lib/api";
import { useAsync } from "../lib/hooks";
import { Async, Badge, Modal, Toggle, toast } from "../components/ui";

export default function Shares() {
  const shares = useAsync(api.shares);
  const datasets = useAsync(api.datasets);
  const [showNew, setShowNew] = useState(false);

  return (
    <>
      <div className="row" style={{ justifyContent: "space-between", marginBottom: 14 }}>
        <p className="muted" style={{ margin: 0 }}>
          Expose datasets over SMB or NFS. Toggling a share reconfigures the mock file server.
        </p>
        <button className="btn primary sm" onClick={() => setShowNew(true)}>+ New share</button>
      </div>

      <div className="card" style={{ padding: 0 }}>
        <Async state={shares}>
          {(list) =>
            list.length === 0 ? (
              <div className="empty">No shares yet.</div>
            ) : (
              <table>
                <thead>
                  <tr><th>Name</th><th>Protocol</th><th>Path</th><th>Access</th><th>Enabled</th><th></th></tr>
                </thead>
                <tbody>
                  {list.map((s) => (
                    <tr key={s.id}>
                      <td><b>{s.name}</b></td>
                      <td><Badge tone={s.kind === "smb" ? "blue" : "accent"}>{s.kind.toUpperCase()}</Badge></td>
                      <td className="mono faint">{s.path}</td>
                      <td>
                        <div className="row" style={{ gap: 6 }}>
                          {s.read_only ? <Badge tone="gray">read-only</Badge> : <Badge tone="green">read/write</Badge>}
                          {s.guest_ok && <Badge tone="yellow">guest</Badge>}
                        </div>
                      </td>
                      <td>
                        <Toggle
                          checked={s.enabled}
                          onChange={async (v) => {
                            try { await api.patchShare(s.id, { enabled: v }); shares.reload(); }
                            catch (e) { toast(String((e as Error).message), true); }
                          }}
                        />
                      </td>
                      <td className="right">
                        <button
                          className="btn sm ghost danger"
                          onClick={async () => {
                            if (!confirm(`Delete share "${s.name}"?`)) return;
                            try { await api.deleteShare(s.id); toast(`Deleted ${s.name}`); shares.reload(); }
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
            )
          }
        </Async>
      </div>

      {showNew && (
        <NewShareModal
          datasets={datasets.data ?? []}
          onClose={() => setShowNew(false)}
          onDone={() => { setShowNew(false); shares.reload(); }}
        />
      )}
    </>
  );
}

function NewShareModal({
  datasets, onClose, onDone,
}: {
  datasets: Dataset[];
  onClose: () => void;
  onDone: () => void;
}) {
  const [name, setName] = useState("");
  const [kind, setKind] = useState<ShareKind>("smb");
  const [datasetId, setDatasetId] = useState(datasets[0]?.id ?? "");
  const [readOnly, setReadOnly] = useState(false);
  const [guestOk, setGuestOk] = useState(false);
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    setBusy(true);
    try {
      await api.createShare({ name, kind, dataset_id: datasetId, read_only: readOnly, guest_ok: guestOk });
      toast(`Created share ${name}`);
      onDone();
    } catch (e) {
      toast(String((e as Error).message), true);
      setBusy(false);
    }
  };

  return (
    <Modal title="New share" onClose={onClose}>
      <label className="field">
        <span>Share name</span>
        <input value={name} onChange={(e) => setName(e.target.value)} placeholder="e.g. Backups" />
      </label>
      <div className="row" style={{ gap: 12 }}>
        <label className="field" style={{ flex: 1 }}>
          <span>Protocol</span>
          <select value={kind} onChange={(e) => setKind(e.target.value as ShareKind)}>
            <option value="smb">SMB / CIFS</option>
            <option value="nfs">NFS</option>
          </select>
        </label>
        <label className="field" style={{ flex: 2 }}>
          <span>Dataset</span>
          <select value={datasetId} onChange={(e) => setDatasetId(e.target.value)}>
            {datasets.map((d) => <option key={d.id} value={d.id}>{d.name} — {d.path}</option>)}
          </select>
        </label>
      </div>
      <div className="row" style={{ gap: 20, marginBottom: 4 }}>
        <label className="checkbox">
          <input type="checkbox" checked={readOnly} onChange={(e) => setReadOnly(e.target.checked)} /> Read-only
        </label>
        <label className="checkbox">
          <input type="checkbox" checked={guestOk} onChange={(e) => setGuestOk(e.target.checked)} /> Allow guests
        </label>
      </div>
      <div className="actions">
        <button className="btn ghost" onClick={onClose}>Cancel</button>
        <button className="btn primary" disabled={busy || !name || !datasetId} onClick={submit}>Create</button>
      </div>
    </Modal>
  );
}
