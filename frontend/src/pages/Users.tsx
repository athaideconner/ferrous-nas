import { useState } from "react";
import { api, type Group } from "../lib/api";
import { useAsync } from "../lib/hooks";
import { Async, Badge, Modal, toast } from "../components/ui";

export default function Users() {
  const users = useAsync(api.users);
  const groups = useAsync(api.groups);
  const [showNewUser, setShowNewUser] = useState(false);
  const [showNewGroup, setShowNewGroup] = useState(false);

  const reloadBoth = () => {
    users.reload();
    groups.reload();
  };

  return (
    <div className="grid cols-2">
      <div>
        <div className="row" style={{ justifyContent: "space-between", marginBottom: 12 }}>
          <div className="section-title" style={{ margin: 0 }}>Users</div>
          <button className="btn primary sm" onClick={() => setShowNewUser(true)}>+ Add user</button>
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
                            try { await api.deleteUser(u.id); toast(`Deleted ${u.username}`); reloadBoth(); }
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
        <div className="row" style={{ justifyContent: "space-between", marginBottom: 12 }}>
          <div className="section-title" style={{ margin: 0 }}>Groups</div>
          <button className="btn sm" onClick={() => setShowNewGroup(true)}>+ New group</button>
        </div>
        <div className="card" style={{ padding: 0 }}>
          <Async state={groups}>
            {(list) =>
              list.length === 0 ? (
                <div className="empty">No groups yet.</div>
              ) : (
                <table>
                  <thead><tr><th>Group</th><th>Members</th><th></th></tr></thead>
                  <tbody>
                    {list.map((g) => (
                      <tr key={g.id}>
                        <td><b>{g.name}</b></td>
                        <td className="faint">{g.members.join(", ") || "—"}</td>
                        <td className="right">
                          <button
                            className="btn sm ghost danger"
                            disabled={g.members.length > 0}
                            title={g.members.length > 0 ? "Remove its members first" : undefined}
                            onClick={async () => {
                              if (!confirm(`Delete group "${g.name}"?`)) return;
                              try { await api.deleteGroup(g.id); toast(`Deleted ${g.name}`); groups.reload(); }
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
      </div>

      {showNewUser && (
        <NewUserModal
          groups={groups.data ?? []}
          onClose={() => setShowNewUser(false)}
          onDone={() => { setShowNewUser(false); reloadBoth(); }}
        />
      )}
      {showNewGroup && (
        <NewGroupModal
          onClose={() => setShowNewGroup(false)}
          onDone={() => { setShowNewGroup(false); groups.reload(); }}
        />
      )}
    </div>
  );
}

function NewUserModal({
  groups, onClose, onDone,
}: {
  groups: Group[];
  onClose: () => void;
  onDone: () => void;
}) {
  const [username, setUsername] = useState("");
  const [fullName, setFullName] = useState("");
  const [password, setPassword] = useState("");
  const [isAdmin, setIsAdmin] = useState(false);
  const [selected, setSelected] = useState<Set<string>>(new Set());
  const [busy, setBusy] = useState(false);

  const toggleGroup = (name: string) => {
    const next = new Set(selected);
    next.has(name) ? next.delete(name) : next.add(name);
    setSelected(next);
  };

  const submit = async () => {
    setBusy(true);
    try {
      await api.createUser({
        username,
        full_name: fullName,
        password,
        is_admin: isAdmin,
        groups: [...selected],
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
      <label className="field">
        <span>Password</span>
        <input
          type="password"
          value={password}
          autoComplete="new-password"
          onChange={(e) => setPassword(e.target.value)}
          placeholder="At least 8 characters"
        />
      </label>
      <label className="checkbox" style={{ marginBottom: 12 }}>
        <input type="checkbox" checked={isAdmin} onChange={(e) => setIsAdmin(e.target.checked)} /> Administrator
      </label>
      <label className="field">
        <span>Groups</span>
      </label>
      {groups.length === 0 ? (
        <div className="faint" style={{ marginBottom: 12 }}>No groups yet — create one first, or leave this user unassigned.</div>
      ) : (
        <div className="vstack" style={{ gap: 6, marginBottom: 4 }}>
          {groups.map((g) => (
            <label key={g.id} className="checkbox">
              <input type="checkbox" checked={selected.has(g.name)} onChange={() => toggleGroup(g.name)} />
              {g.name}
            </label>
          ))}
        </div>
      )}
      <div className="actions">
        <button className="btn ghost" onClick={onClose}>Cancel</button>
        <button className="btn primary" disabled={busy || !username || password.length < 8} onClick={submit}>
          Create
        </button>
      </div>
    </Modal>
  );
}

function NewGroupModal({ onClose, onDone }: { onClose: () => void; onDone: () => void }) {
  const [name, setName] = useState("");
  const [busy, setBusy] = useState(false);

  const submit = async () => {
    setBusy(true);
    try {
      await api.createGroup(name);
      toast(`Created group ${name}`);
      onDone();
    } catch (e) {
      toast(String((e as Error).message), true);
      setBusy(false);
    }
  };

  return (
    <Modal title="New group" onClose={onClose}>
      <label className="field">
        <span>Group name</span>
        <input value={name} onChange={(e) => setName(e.target.value.replace(/\s/g, ""))} placeholder="e.g. engineers" />
      </label>
      <div className="actions">
        <button className="btn ghost" onClick={onClose}>Cancel</button>
        <button className="btn primary" disabled={busy || !name} onClick={submit}>Create</button>
      </div>
    </Modal>
  );
}
