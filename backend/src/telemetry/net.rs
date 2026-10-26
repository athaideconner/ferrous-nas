//! Real network information for [`super::linux::LinuxTelemetry`]: the
//! machine's interfaces (`ip -j addr` plus `/sys/class/net`), its default
//! gateway (`ip -j route`) and DNS servers (systemd-resolved's upstream list).
//! Read-only, like the rest of telemetry.

use std::fs;
use std::path::Path;
use std::process::Command;

use crate::models::{NetInterface, NetworkSummary};

pub fn interfaces() -> Vec<NetInterface> {
    let Some(json) = ip_json(&["addr", "show"]) else { return vec![] };
    parse_addrs(&json)
        .into_iter()
        .map(|mut n| {
            let sys = Path::new("/sys/class/net").join(&n.name);
            n.kind = kind_of(&sys, &n.kind);
            let num = |f: &str| fs::read_to_string(sys.join(f)).ok().and_then(|s| s.trim().parse::<i64>().ok());
            // Virtual NICs report -1 (or nothing): no meaningful link speed.
            n.speed_mbps = num("speed").filter(|s| *s > 0).unwrap_or(0) as u32;
            n.rx_bytes = num("statistics/rx_bytes").unwrap_or(0) as u64;
            n.tx_bytes = num("statistics/tx_bytes").unwrap_or(0) as u64;
            n
        })
        // Unused virtual devices (e.g. the kernel's ifb traffic-shaping
        // ones): down, no address, nothing to show.
        .filter(|n| !(n.kind == "virtual" && !n.up && n.ipv4.is_none() && n.ipv6.is_none()))
        .collect()
}

pub fn summary() -> NetworkSummary {
    let hostname = fs::read_to_string("/proc/sys/kernel/hostname").unwrap_or_default().trim().to_string();
    let gateway = ip_json(&["route", "show", "default"]).and_then(|j| parse_gateway(&j));
    // resolved's list of the real upstream servers; /etc/resolv.conf as a
    // fallback (on the image it's a link to the same file).
    let dns = ["/run/systemd/resolve/resolv.conf", "/etc/resolv.conf"]
        .iter()
        .find_map(|p| fs::read_to_string(p).ok())
        .map(|s| parse_nameservers(&s))
        .unwrap_or_default();
    NetworkSummary { hostname, gateway, dns }
}

fn ip_json(args: &[&str]) -> Option<serde_json::Value> {
    let out = Command::new("ip").arg("-j").args(args).output().ok()?;
    if !out.status.success() {
        return None;
    }
    serde_json::from_slice(&out.stdout).ok()
}

/// Interfaces worth showing, from `ip -j addr show`: everything but
/// loopback and the per-container `veth` pairs Docker creates (one per
/// running app — noise next to the real NICs and bridges). `kind` is left as
/// ip's link type, refined from sysfs by the caller.
pub(crate) fn parse_addrs(json: &serde_json::Value) -> Vec<NetInterface> {
    let Some(links) = json.as_array() else { return vec![] };
    let mut out = Vec::new();
    for l in links {
        let s = |k: &str| l.get(k).and_then(|v| v.as_str()).unwrap_or("").to_string();
        let name = s("ifname");
        let link_type = s("link_type");
        if name.is_empty() || link_type == "loopback" || name.starts_with("veth") {
            continue;
        }
        let flags: Vec<&str> = l
            .get("flags")
            .and_then(|f| f.as_array())
            .map(|a| a.iter().filter_map(|v| v.as_str()).collect())
            .unwrap_or_default();
        // "UNKNOWN" is what drivers without carrier detection report.
        let up = match s("operstate").as_str() {
            "UP" => true,
            "UNKNOWN" => flags.contains(&"LOWER_UP"),
            _ => false,
        };
        let addrs: Vec<&serde_json::Value> =
            l.get("addr_info").and_then(|a| a.as_array()).map(|a| a.iter().collect()).unwrap_or_default();
        let cidr = |a: &serde_json::Value| {
            format!(
                "{}/{}",
                a.get("local").and_then(|v| v.as_str()).unwrap_or(""),
                a.get("prefixlen").and_then(|v| v.as_u64()).unwrap_or(0)
            )
        };
        let family = |a: &&&serde_json::Value, f: &str| a.get("family").and_then(|v| v.as_str()) == Some(f);
        let scope = |a: &&&serde_json::Value, sc: &str| a.get("scope").and_then(|v| v.as_str()) == Some(sc);
        let ipv4 = addrs.iter().find(|a| family(a, "inet")).map(|a| cidr(a));
        // A routable IPv6 address if there is one, else the link-local one.
        let ipv6 = addrs
            .iter()
            .find(|a| family(a, "inet6") && scope(a, "global"))
            .or_else(|| addrs.iter().find(|a| family(a, "inet6")))
            .map(|a| cidr(a));
        // ip marks addresses with a lease (DHCP) as "dynamic".
        let dhcp = addrs
            .iter()
            .any(|a| family(&a, "inet") && a.get("dynamic").and_then(|v| v.as_bool()) == Some(true));
        out.push(NetInterface {
            name,
            mac: s("address"),
            ipv4,
            ipv6,
            kind: link_type,
            up,
            speed_mbps: 0,
            rx_bytes: 0,
            tx_bytes: 0,
            dhcp,
        });
    }
    out
}

/// What sort of interface `sys` (its `/sys/class/net/<name>` directory) is.
fn kind_of(sys: &Path, link_type: &str) -> String {
    let kind = if sys.join("bridge").exists() {
        "bridge"
    } else if sys.join("bonding").exists() {
        "bond"
    } else if sys.join("wireless").exists() || sys.join("phy80211").exists() {
        "wireless"
    } else if link_type == "ether" && sys.join("device").exists() {
        "ethernet"
    } else {
        "virtual"
    };
    kind.to_string()
}

/// The gateway of the first default route in `ip -j route show default`.
pub(crate) fn parse_gateway(json: &serde_json::Value) -> Option<String> {
    json.as_array()?
        .iter()
        .find_map(|r| r.get("gateway").and_then(|v| v.as_str()).map(str::to_string))
}

pub(crate) fn parse_nameservers(resolv_conf: &str) -> Vec<String> {
    resolv_conf
        .lines()
        .filter_map(|l| l.trim().strip_prefix("nameserver"))
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn addrs_skip_loopback_and_container_veths() {
        let j = json!([
            {"ifname":"lo","link_type":"loopback","operstate":"UNKNOWN","flags":["LOOPBACK","UP","LOWER_UP"],"address":"00:00:00:00:00:00",
             "addr_info":[{"family":"inet","local":"127.0.0.1","prefixlen":8,"scope":"host"}]},
            {"ifname":"enp1s0","link_type":"ether","operstate":"UP","flags":["UP","LOWER_UP"],"address":"52:54:00:6a:52:79",
             "addr_info":[
                {"family":"inet","local":"192.168.122.69","prefixlen":24,"scope":"global","dynamic":true},
                {"family":"inet6","local":"fe80::5054:ff:fe6a:5279","prefixlen":64,"scope":"link"},
                {"family":"inet6","local":"2001:db8::69","prefixlen":64,"scope":"global","dynamic":true}]},
            {"ifname":"docker0","link_type":"ether","operstate":"DOWN","flags":["NO-CARRIER","UP"],"address":"02:42:ac:11:00:01",
             "addr_info":[{"family":"inet","local":"172.17.0.1","prefixlen":16,"scope":"global"}]},
            {"ifname":"veth12ab","link_type":"ether","operstate":"UP","flags":["UP"],"address":"aa:bb:cc:dd:ee:ff","addr_info":[]}
        ]);
        let n = parse_addrs(&j);
        assert_eq!(n.iter().map(|i| i.name.as_str()).collect::<Vec<_>>(), ["enp1s0", "docker0"]);
        assert_eq!(n[0].ipv4.as_deref(), Some("192.168.122.69/24"));
        assert_eq!(n[0].ipv6.as_deref(), Some("2001:db8::69/64"), "global IPv6 preferred over link-local");
        assert!(n[0].up && n[0].dhcp);
        assert!(!n[1].up && !n[1].dhcp, "docker0: no carrier, static address");
    }

    #[test]
    fn gateway_and_dns() {
        let r = json!([{"dst":"default","gateway":"192.168.122.1","dev":"enp1s0","protocol":"dhcp"}]);
        assert_eq!(parse_gateway(&r).as_deref(), Some("192.168.122.1"));
        assert_eq!(parse_gateway(&json!([])), None);
        let conf = "# generated\nnameserver 192.168.122.1\nnameserver  1.1.1.1\nsearch lan\n";
        assert_eq!(parse_nameservers(conf), ["192.168.122.1", "1.1.1.1"]);
    }
}
