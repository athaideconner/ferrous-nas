import { api, fmtUptime } from "../lib/api";
import { useAsync } from "../lib/hooks";
import { Async, Badge, toast } from "../components/ui";

export default function System() {
  const sys = useAsync(api.system);
  const alerts = useAsync(api.alerts);

  const power = async (fn: () => Promise<unknown>, label: string) => {
    if (!confirm(`${label} the NAS? (mock — nothing actually happens)`)) return;
    try { await fn(); toast(`${label} requested (mock)`); }
    catch (e) { toast(String((e as Error).message), true); }
  };

  return (
    <>
      <Async state={sys}>
        {(s) => (
          <div className="grid cols-2">
            <div className="card">
              <h2>System information</h2>
              <div className="sub">read-only</div>
              <table>
                <tbody>
                  <tr><td className="faint">Product</td><td className="right">{s.product}</td></tr>
                  <tr><td className="faint">Version</td><td className="right mono">{s.version}</td></tr>
                  <tr><td className="faint">Hostname</td><td className="right mono">{s.hostname}</td></tr>
                  <tr><td className="faint">Kernel</td><td className="right mono">{s.kernel}</td></tr>
                  <tr><td className="faint">CPU</td><td className="right">{s.cpu.model}</td></tr>
                  <tr><td className="faint">Uptime</td><td className="right">{fmtUptime(s.uptime_secs)}</td></tr>
                </tbody>
              </table>
            </div>

            <div className="card">
              <h2>Power</h2>
              <div className="sub">these actions are simulated</div>
              <div className="vstack" style={{ gap: 10 }}>
                <button className="btn" onClick={() => power(api.reboot, "Reboot")}>🔄 Reboot</button>
                <button className="btn danger" onClick={() => power(api.shutdown, "Shut down")}>⏻ Shut down</button>
              </div>
              <div className="faint" style={{ marginTop: 14, fontSize: 12 }}>
                In a real build these would call into systemd. Here they return an acknowledgement only.
              </div>
            </div>
          </div>
        )}
      </Async>

      <div className="section-title">Notifications</div>
      <div className="card" style={{ padding: 0 }}>
        <Async state={alerts}>
          {(list) => (
            <table>
              <thead><tr><th>Level</th><th>Title</th><th>Detail</th><th></th></tr></thead>
              <tbody>
                {list.map((a) => (
                  <tr key={a.id} style={{ opacity: a.acknowledged ? 0.55 : 1 }}>
                    <td><Badge tone={a.level === "critical" ? "red" : a.level === "warning" ? "yellow" : "blue"}>{a.level}</Badge></td>
                    <td><b>{a.title}</b></td>
                    <td className="faint">{a.message}</td>
                    <td className="right">
                      {a.acknowledged ? (
                        <span className="faint">acknowledged</span>
                      ) : (
                        <button
                          className="btn sm"
                          onClick={async () => {
                            try { await api.ackAlert(a.id); alerts.reload(); }
                            catch (e) { toast(String((e as Error).message), true); }
                          }}
                        >
                          Acknowledge
                        </button>
                      )}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </Async>
      </div>
    </>
  );
}
