import { api, fmtBytes } from "../lib/api";
import { useAsync } from "../lib/hooks";
import { Async, Badge } from "../components/ui";

export default function Network() {
  const ifaces = useAsync(api.interfaces, 5000);

  return (
    <Async state={ifaces}>
      {(list) => (
        <div className="grid cols-2">
          {list.map((n) => (
            <div key={n.name} className="card">
              <div className="card-head">
                <div className="vstack">
                  <h2>{n.name}</h2>
                  <span className="faint">{n.kind}{n.speed_mbps ? ` · ${n.speed_mbps} Mbps` : ""}</span>
                </div>
                <Badge tone={n.up ? "green" : "gray"}>{n.up ? "up" : "down"}</Badge>
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
      )}
    </Async>
  );
}
