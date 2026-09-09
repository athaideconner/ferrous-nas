import { useState } from "react";
import { api } from "../lib/api";
import { useAsync } from "../lib/hooks";
import { Async, Badge, Modal, toast } from "../components/ui";

export default function Users() {
  const users = useAsync(api.users);
  const groups = useAsync(api.groups);
  const [showNew, setShowNew] = useState(false);

  return (
    <div className="grid cols-2">
      <div>
        <div className="row" style={{ justifyContent: "space-between", marginBottom: 12 }}>
          <div className="section-title" style={{ margin: 0 }}>Users</div>
          <button className="btn primary sm" onClick={() => setShowNew(true)}>+ Add user</button>
        </div>
        <div className="card" style={{ padding: 0 }}>
          <Async state={users}>
            {(list) => (
              <table>
                <thead><tr><th>User</th><th>Groups</th><th>Role</th><th></th></tr></thead>
                <tbody>
                  {list.map((u) => (
                    <tr key={u.id}>
                      <td>
                        <div className="vstack">
                          <b>{u.username}</b>
                          <span className="faint">{u.full_name}</span>
                        </div>
                      </td>
                      <td>
                        <div className="row" style={{ gap: 5, flexWrap: "wrap" }}>
                          {u.groups.map((g) => <Badge key={g} tone="gray">{g}</Badge>)}
                        </div>
                      </td>
                      <td>{u.is_admin ? <Badge tone="accent">admin</Badge> : <span className="faint">user</span>}</td>
                      <td className="right">
                        <button
                          className="btn sm ghost danger"
                          onClick={async () => {
                            if (!confirm(`Delete user "${u.username}"?`)) return;
                            try { await api.deleteUser(u.id); toast(`Deleted ${u.username}`); users.reload(); }
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
      </div>

      <div>
        <div className="section-title" style={{ marginTop: 0 }}>Groups</div>
        <div className="card" style={{ padding: 0 }}>
          <Async state={groups}>
            {(list) => (
              <table>
                <thead><tr><th>Group</th><th>Members</th></tr></thead>
                <tbody>
                  {list.map((g) => (
                    <tr key={g.id}>
                      <td><b>{g.name}</b></td>
                      <td className="faint">{g.members.join(", ") || "—"}</td>
                    </tr>
                  ))}
                </tbody>
              </table>
            )}
          </Async>
        </div>
      </div>

      {showNew && <NewUserModal onClose={() => setShowNew(false)} onDone={() => { setShowNew(false); users.reload(); }} />}
    </div>
  );
}

function NewUserModal({ onClose, onDone }: { onClose: () => void; onDone: () => void }) {
  const [username, setUsername] = useState("");
  const [fullName, setFullName] = useState("");
  const [isAdmin, setIsAdmin] = useState(false);
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    setBusy(true);
    try {
      await api.createUser({
        username,
        full_name: fullName,
        is_admin: isAdmin,
        groups: isAdmin ? ["admins"] : ["family"],
      });
      toast(`Created user ${username}`);
      onDone();
    } catch (e) {
      toast(String((e as Error).message), true);
      setBusy(false);
    }
  };

  return (
    <Modal title="Add user" onClose={onClose}>
      <label className="field">
        <span>Username</span>
        <input value={username} onChange={(e) => setUsername(e.target.value.replace(/\s/g, ""))} placeholder="e.g. alex" />
      </label>
      <label className="field">
        <span>Full name</span>
        <input value={fullName} onChange={(e) => setFullName(e.target.value)} placeholder="Alex Doe" />
      </label>
      <label className="checkbox" style={{ marginBottom: 4 }}>
        <input type="checkbox" checked={isAdmin} onChange={(e) => setIsAdmin(e.target.checked)} /> Administrator
      </label>
      <div className="actions">
        <button className="btn ghost" onClick={onClose}>Cancel</button>
        <button className="btn primary" disabled={busy || !username} onClick={submit}>Create</button>
      </div>
    </Modal>
  );
}
