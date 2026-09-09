import { api, fmtBytes, fmtUptime } from "../lib/api";
import { useAsync } from "../lib/hooks";
import { Async, Badge, Meter, Ring, Sparkline, StatTile } from "../components/ui";

export default function Dashboard() {
  const sys = useAsync(api.system, 3000);
  const stats = useAsync(() => api.stats(60), 3000);
  const pools = useAsync(api.pools, 8000);
  const apps = useAsync(api.apps, 8000);
  const alerts = useAsync(api.alerts, 8000);

  return (
    <Async state={sys}>
      {(s) => {
        const memPct = (s.memory.used_bytes / s.memory.total_bytes) * 100;
        const running = apps.data?.filter((a) => a.state === "running").length ?? 0;
        const unacked = alerts.data?.filter((a) => !a.acknowledged).length ?? 0;
        const totalCap = pools.data?.reduce((a, p) => a + p.size_bytes, 0) ?? 0;
        const usedCap = pools.data?.reduce((a, p) => a + p.used_bytes, 0) ?? 0;

        return (
          <>
            <div className="grid cols-4">
              <StatTile
                label="Uptime"
                value={fmtUptime(s.uptime_secs).split(" ")[0]}
                unit="days"
                foot={fmtUptime(s.uptime_secs)}
              />
              <StatTile
                label="Storage used"
                value={fmtBytes(usedCap)}
                foot={`of ${fmtBytes(totalCap)} across ${pools.data?.length ?? 0} pools`}
              />
              <StatTile label="Apps running" value={running} foot={`${apps.data?.length ?? 0} installed`} />
              <StatTile
                label="Alerts"
                value={unacked}
                foot={unacked ? "need attention" : "all acknowledged"}
              />
            </div>

            <div className="grid cols-3" style={{ marginTop: 16 }}>
              <div className="card">
                <div className="card-head">
                  <h2>CPU</h2>
                  <Badge tone="gray">{s.cpu.temp_c.toFixed(0)}°C</Badge>
                </div>
                <div className="hstack" style={{ gap: 18 }}>
                  <Ring pct={s.cpu.usage_percent} sub="load" />
                  <div className="vstack">
                    <div className="muted">{s.cpu.model}</div>
                    <div className="faint">
                      {s.cpu.cores} cores · {s.cpu.threads} threads
                    </div>
                    <div className="faint">load avg {s.load_avg.map((l) => l.toFixed(2)).join("  ")}</div>
                  </div>
                </div>
              </div>

              <div className="card">
                <div className="card-head">
                  <h2>Memory</h2>
                  <Badge tone="gray">{fmtBytes(s.memory.total_bytes)}</Badge>
                </div>
                <div className="hstack" style={{ gap: 18 }}>
                  <Ring pct={memPct} sub="used" />
                  <div className="vstack" style={{ flex: 1 }}>
                    <div className="muted">
                      {fmtBytes(s.memory.used_bytes)} / {fmtBytes(s.memory.total_bytes)}
                    </div>
                    <div className="faint" style={{ marginBottom: 8 }}>
                      swap {fmtBytes(s.memory.swap_used_bytes)} / {fmtBytes(s.memory.swap_total_bytes)}
                    </div>
                    <Meter pct={(s.memory.swap_used_bytes / s.memory.swap_total_bytes) * 100} tone="" />
                  </div>
                </div>
              </div>

              <div className="card">
                <h2>Live throughput</h2>
                <div className="sub">last 60 samples</div>
                {stats.data && (
                  <div className="vstack" style={{ gap: 12 }}>
                    <ThroughputRow
                      label="Network"
                      color="var(--blue)"
                      data={stats.data.map((p) => p.net_rx_mbps)}
                      value={`${stats.data.at(-1)?.net_rx_mbps.toFixed(0)} Mb/s`}
                    />
                    <ThroughputRow
                      label="Disk"
                      color="var(--accent)"
                      data={stats.data.map((p) => p.disk_read_mbps)}
                      value={`${stats.data.at(-1)?.disk_read_mbps.toFixed(0)} MB/s`}
                    />
                  </div>
                )}
              </div>
            </div>

            <div className="grid cols-2" style={{ marginTop: 16 }}>
              <div className="card">
                <h2>Storage pools</h2>
                <div className="sub">capacity utilisation</div>
                {pools.data?.map((p) => {
                  const pct = (p.used_bytes / p.size_bytes) * 100;
                  return (
                    <div key={p.id} style={{ marginBottom: 14 }}>
                      <div className="row" style={{ justifyContent: "space-between", marginBottom: 6 }}>
                        <span>
                          <b>{p.name}</b> <span className="faint">· {p.raid_level}</span>
                        </span>
                        <span className="faint">
                          {fmtBytes(p.used_bytes)} / {fmtBytes(p.size_bytes)} ({pct.toFixed(0)}%)
                        </span>
                      </div>
                      <Meter pct={pct} />
                    </div>
                  );
                })}
              </div>

              <div className="card">
                <h2>Recent alerts</h2>
                <div className="sub">system notifications</div>
                {alerts.data?.length ? (
                  alerts.data.slice(0, 5).map((a) => (
                    <div key={a.id} className="row" style={{ alignItems: "flex-start", gap: 10, marginBottom: 12 }}>
                      <Badge tone={a.level === "critical" ? "red" : a.level === "warning" ? "yellow" : "blue"}>
                        {a.level}
                      </Badge>
                      <div className="vstack">
                        <span style={{ opacity: a.acknowledged ? 0.55 : 1 }}>{a.title}</span>
                        <span className="faint">{a.message}</span>
                      </div>
                    </div>
                  ))
                ) : (
                  <div className="empty">No alerts</div>
                )}
              </div>
            </div>
          </>
        );
      }}
    </Async>
  );
}

function ThroughputRow({ label, color, data, value }: { label: string; color: string; data: number[]; value: string }) {
  return (
    <div>
      <div className="row" style={{ justifyContent: "space-between" }}>
        <span className="faint">{label}</span>
        <span className="mono">{value}</span>
      </div>
      <Sparkline data={data} color={color} />
    </div>
  );
}
