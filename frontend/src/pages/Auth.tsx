import { useState, type FormEvent } from "react";
import { api } from "../lib/api";

/**
 * Sign-in and first-run setup. The two forms are near-identical, so they share
 * one component: setup additionally collects a display name and a confirmation.
 */
export default function Auth({ mode, onDone }: { mode: "login" | "setup"; onDone: () => void }) {
  const setup = mode === "setup";
  const [username, setUsername] = useState("");
  const [fullName, setFullName] = useState("");
  const [password, setPassword] = useState("");
  const [confirm, setConfirm] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setError(null);
    if (setup && password !== confirm) {
      setError("Passwords do not match");
      return;
    }
    setBusy(true);
    try {
      if (setup) await api.auth.setup(username, fullName, password);
      else await api.auth.login(username, password);
      onDone();
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err));
      setBusy(false);
    }
  };

  return (
    <div className="auth-wrap">
      <form className="auth-card" onSubmit={submit}>
        <div className="auth-brand">
          <span className="logo">🗄️</span>
          <div>
            <div className="name">FerrousNAS</div>
            <div className="ver">{setup ? "First-time setup" : "Sign in"}</div>
          </div>
        </div>

        {setup && (
          <p className="faint" style={{ marginTop: 0 }}>
            No administrator exists yet. Create one to secure this system — there is no
            default password.
          </p>
        )}

        <label className="field">
          <span>Username</span>
          <input
            value={username}
            autoFocus
            autoComplete="username"
            onChange={(e) => setUsername(e.target.value.replace(/\s/g, ""))}
            placeholder="e.g. gorav"
          />
        </label>

        {setup && (
          <label className="field">
            <span>Display name</span>
            <input value={fullName} onChange={(e) => setFullName(e.target.value)} placeholder="Optional" />
          </label>
        )}

        <label className="field">
          <span>Password</span>
          <input
            type="password"
            value={password}
            autoComplete={setup ? "new-password" : "current-password"}
            onChange={(e) => setPassword(e.target.value)}
            placeholder={setup ? "At least 8 characters" : ""}
          />
        </label>

        {setup && (
          <label className="field">
            <span>Confirm password</span>
            <input
              type="password"
              value={confirm}
              autoComplete="new-password"
              onChange={(e) => setConfirm(e.target.value)}
            />
          </label>
        )}

        {error && <div className="auth-error">⚠️ {error}</div>}

        <button className="btn primary" type="submit" disabled={busy || !username || !password}>
          {busy ? "Please wait…" : setup ? "Create administrator" : "Sign in"}
        </button>
      </form>
    </div>
  );
}
