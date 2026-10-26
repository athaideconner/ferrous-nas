import { api, fmtBytes } from "../lib/api";
import { useAsync } from "../lib/hooks";
import { Async, Badge } from "../components/ui";

export default function Network() {
  const ifaces = useAsync(api.interfaces, 5000);
  const summary = useAsync(api.networkSummary, 30000);

  return (
    <>
      <div className="card" style={{ marginBottom: 18 }}>
        <Async state={summary}>
          {(s) => (
            <table>
              <tbody>
                <tr><td className="faint">Hostname</td><td className="mono right">{s.hostname || "—"}</td></tr>
                <tr><td className="faint">Default gateway</td><td className="mono right">{s.gateway ?? "—"}</td></tr>
                <tr><td className="faint">DNS servers</td><td className="mono right">{s.dns.length ? s.dns.join(", ") : "—"}</td></tr>
              </tbody>
            </table>
          )}
        </Async>
      </div>

      <Async state={ifaces}>
        {(list) =>
          list.length === 0 ? (
            <div className="empty">No network interfaces found.</div>
          ) : (
            <div className="grid cols-2">
              {list.map((n) => (
                <div key={n.name} className="card">
                  <div className="card-head">
                    <div className="vstack">
                      <h2>{n.name}</h2>
                      <span className="faint">{n.kind}{n.speed_mbps ? ` · ${n.speed_mbps} Mbps` : ""}</span>
                    </div>
                    <div className="row" style={{ gap: 6 }}>
                      {n.ipv4 && <Badge tone="gray">{n.dhcp ? "DHCP" : "static"}</Badge>}
                      <Badge tone={n.up ? "green" : "gray"}>{n.up ? "up" : "down"}</Badge>
                    </div>
                  </div>
                  <table>
                    <tbody>
                      <tr><td className="faint">MAC</td><td className="mono right">{n.mac}</td></tr>
                      <tr><td className="faint">IPv4</td><td className="mono right">{n.ipv4 ?? "—"}</td></tr>
                      <tr><td className="faint">IPv6</td><td className="mono right">{n.ipv6 ?? "—"}</td></tr>
                      <tr><td className="faint">Received</td><td className="right">{fmtBytes(n.rx_bytes)}</td></tr>
                      <tr><td className="faint">Transmitted</td><td className="right">{fmtBytes(n.tx_bytes)}</td></tr>
                    </tbody>
                  </table>
                </div>
              ))}
            </div>
          )
        }
      </Async>
    </>
  );
}
