//! In-memory, mocked datastore.
//!
//! `Store` holds every entity the API serves. It is seeded with believable
//! sample data on boot and mutated by the handlers. Because everything lives
//! behind an `RwLock`, the whole thing is safe to share across the async
//! runtime. Nothing here touches real disks, services, or containers.

use std::sync::Arc;
use std::time::Instant;

use rand::Rng;
use tokio::sync::RwLock;

use crate::models::*;

pub type Db = Arc<RwLock<Store>>;

pub struct Store {
    pub boot: Instant,
    pub hostname: String,
    pub disks: Vec<Disk>,
    pub pools: Vec<Pool>,
    pub datasets: Vec<Dataset>,
    pub shares: Vec<Share>,
    pub catalog: Vec<CatalogApp>,
    pub apps: Vec<InstalledApp>,
    pub users: Vec<User>,
    pub groups: Vec<Group>,
    pub interfaces: Vec<NetInterface>,
    pub alerts: Vec<Alert>,
}

const GB: u64 = 1_000_000_000;
const TB: u64 = 1_000 * GB;

fn now_iso() -> String {
    chrono::Utc::now().to_rfc3339()
}

impl Store {
    pub fn seeded() -> Self {
        let disks = seed_disks();
        let (pools, datasets) = seed_pools_and_datasets(&disks);
        let shares = seed_shares(&datasets);
        let catalog = seed_catalog();
        let apps = seed_apps(&catalog);
        let (users, groups) = seed_users();

        Store {
            boot: Instant::now(),
            hostname: "ferrous-nas".to_string(),
            disks,
            pools,
            datasets,
            shares,
            catalog,
            apps,
            users,
            groups,
            interfaces: seed_interfaces(),
            alerts: seed_alerts(),
        }
    }

    pub fn uptime_secs(&self) -> u64 {
        // Pretend the box has been up for a base amount plus real process time,
        // so the number is both large (realistic) and visibly ticking.
        612_345 + self.boot.elapsed().as_secs()
    }

    pub fn system_info(&self) -> SystemInfo {
        let mut rng = rand::thread_rng();
        let total = 64 * GB;
        let used = 18 * GB + rng.gen_range(0..(6 * GB));
        SystemInfo {
            hostname: self.hostname.clone(),
            product: "FerrousNAS".to_string(),
            version: crate::VERSION.to_string(),
            kernel: "6.6.0-ferrous-amd64".to_string(),
            uptime_secs: self.uptime_secs(),
            cpu: CpuInfo {
                model: "AMD Ryzen 5 5600 (mock)".to_string(),
                cores: 6,
                threads: 12,
                usage_percent: rng.gen_range(4.0..38.0),
                temp_c: rng.gen_range(38.0..52.0),
            },
            memory: MemoryInfo {
                total_bytes: total,
                used_bytes: used,
                swap_total_bytes: 8 * GB,
                swap_used_bytes: rng.gen_range(0..(1 * GB)),
            },
            load_avg: [
                rng.gen_range(0.2..1.5),
                rng.gen_range(0.2..1.2),
                rng.gen_range(0.2..0.9),
            ],
        }
    }

    /// A synthetic rolling time-series (last `points` minutes, one per minute).
    pub fn stats_history(&self, points: usize) -> Vec<StatPoint> {
        let mut rng = rand::thread_rng();
        let mut out = Vec::with_capacity(points);
        for i in 0..points {
            let t = -((points as i64 - 1 - i as i64) * 60);
            // Gentle sine-ish base + jitter so the chart looks alive.
            let phase = (i as f32) / 6.0;
            out.push(StatPoint {
                t,
                cpu_percent: (18.0 + 12.0 * phase.sin() + rng.gen_range(-4.0..4.0)).clamp(1.0, 100.0),
                mem_percent: (34.0 + 3.0 * (phase / 2.0).cos() + rng.gen_range(-2.0..2.0)).clamp(1.0, 100.0),
                net_rx_mbps: (120.0 + 90.0 * phase.sin() + rng.gen_range(-30.0..30.0)).max(0.0),
                net_tx_mbps: (60.0 + 40.0 * (phase / 1.5).cos() + rng.gen_range(-20.0..20.0)).max(0.0),
                disk_read_mbps: (200.0 + 150.0 * (phase / 1.3).sin() + rng.gen_range(-40.0..40.0)).max(0.0),
                disk_write_mbps: (90.0 + 70.0 * phase.cos() + rng.gen_range(-30.0..30.0)).max(0.0),
            });
        }
        out
    }
}

fn seed_disks() -> Vec<Disk> {
    let mk = |device: &str, model: &str, size: u64, kind: DiskKind, smart: SmartStatus, hours: u32, temp: f32, pool: Option<&str>| Disk {
        id: short_id("disk"),
        device: device.to_string(),
        model: model.to_string(),
        serial: format!("SN{}", short_id("").trim_start_matches('-').to_uppercase()),
        size_bytes: size,
        kind,
        temp_c: temp,
        smart,
        power_on_hours: hours,
        pool_id: pool.map(|s| s.to_string()),
    };
    // pool ids are assigned in seed_pools_and_datasets; leave None here and
    // wire them up there.
    vec![
        mk("sda", "WD Red Plus WD40EFPX", 4 * TB, DiskKind::Hdd, SmartStatus::Passed, 21_400, 36.0, None),
        mk("sdb", "WD Red Plus WD40EFPX", 4 * TB, DiskKind::Hdd, SmartStatus::Passed, 21_390, 37.0, None),
        mk("sdc", "WD Red Plus WD40EFPX", 4 * TB, DiskKind::Hdd, SmartStatus::Warning, 33_120, 41.0, None),
        mk("sdd", "Seagate IronWolf ST8000VN004", 8 * TB, DiskKind::Hdd, SmartStatus::Passed, 9_800, 35.0, None),
        mk("nvme0n1", "Samsung 980 PRO 1TB", 1 * TB, DiskKind::Nvme, SmartStatus::Passed, 4_200, 44.0, None),
        mk("nvme1n1", "Samsung 980 PRO 1TB", 1 * TB, DiskKind::Nvme, SmartStatus::Passed, 4_180, 45.0, None),
    ]
}

fn seed_pools_and_datasets(disks: &[Disk]) -> (Vec<Pool>, Vec<Dataset>) {
    // tank: raidz1 across the three 4TB reds. cache: mirror across NVMe.
    let tank_id = short_id("pool");
    let cache_id = short_id("pool");

    let tank_disks: Vec<String> = disks
        .iter()
        .filter(|d| ["sda", "sdb", "sdc"].contains(&d.device.as_str()))
        .map(|d| d.id.clone())
        .collect();
    let cache_disks: Vec<String> = disks
        .iter()
        .filter(|d| d.device.starts_with("nvme"))
        .map(|d| d.id.clone())
        .collect();

    let pools = vec![
        Pool {
            id: tank_id.clone(),
            name: "tank".to_string(),
            raid_level: RaidLevel::Raidz1,
            status: PoolStatus::Online,
            size_bytes: 8 * TB, // ~usable after parity
            used_bytes: 5_120 * GB,
            disk_ids: tank_disks,
            scrub_progress: None,
        },
        Pool {
            id: cache_id.clone(),
            name: "flash".to_string(),
            raid_level: RaidLevel::Mirror,
            status: PoolStatus::Online,
            size_bytes: 1 * TB,
            used_bytes: 260 * GB,
            disk_ids: cache_disks,
            scrub_progress: None,
        },
    ];

    let ds = |pool_id: &str, name: &str, used: u64, quota: Option<u64>, comp: bool| Dataset {
        id: short_id("ds"),
        pool_id: pool_id.to_string(),
        name: name.to_string(),
        path: format!("/mnt/{}/{}", if pool_id == tank_id { "tank" } else { "flash" }, name),
        used_bytes: used,
        quota_bytes: quota,
        compression: comp,
    };

    let datasets = vec![
        ds(&tank_id, "media", 3_800 * GB, None, true),
        ds(&tank_id, "documents", 210 * GB, Some(500 * GB), true),
        ds(&tank_id, "backups", 980 * GB, Some(2 * TB), true),
        ds(&tank_id, "photos", 330 * GB, None, false),
        ds(&cache_id, "appdata", 180 * GB, None, true),
    ];

    (pools, datasets)
}

fn seed_shares(datasets: &[Dataset]) -> Vec<Share> {
    let find = |name: &str| datasets.iter().find(|d| d.name == name).unwrap();
    let media = find("media");
    let docs = find("documents");
    let backups = find("backups");
    let photos = find("photos");

    vec![
        Share {
            id: short_id("share"),
            name: "Media".to_string(),
            kind: ShareKind::Smb,
            dataset_id: media.id.clone(),
            path: media.path.clone(),
            enabled: true,
            read_only: false,
            guest_ok: true,
            allowed_users: vec![],
        },
        Share {
            id: short_id("share"),
            name: "Documents".to_string(),
            kind: ShareKind::Smb,
            dataset_id: docs.id.clone(),
            path: docs.path.clone(),
            enabled: true,
            read_only: false,
            guest_ok: false,
            allowed_users: vec!["gorav".to_string()],
        },
        Share {
            id: short_id("share"),
            name: "TimeMachine".to_string(),
            kind: ShareKind::Smb,
            dataset_id: backups.id.clone(),
            path: backups.path.clone(),
            enabled: true,
            read_only: false,
            guest_ok: false,
            allowed_users: vec!["gorav".to_string()],
        },
        Share {
            id: short_id("share"),
            name: "photos-nfs".to_string(),
            kind: ShareKind::Nfs,
            dataset_id: photos.id.clone(),
            path: photos.path.clone(),
            enabled: false,
            read_only: true,
            guest_ok: false,
            allowed_users: vec![],
        },
    ]
}

fn seed_catalog() -> Vec<CatalogApp> {
    let a = |name: &str, tagline: &str, desc: &str, icon: &str, cat: &str, image: &str, port: u16| CatalogApp {
        id: name.to_lowercase().replace(' ', "-"),
        name: name.to_string(),
        tagline: tagline.to_string(),
        description: desc.to_string(),
        icon: icon.to_string(),
        category: cat.to_string(),
        image: image.to_string(),
        default_port: port,
    };
    vec![
        a("Jellyfin", "Media server", "Stream your movies, shows and music to any device. The free software media system.", "🎬", "Media", "jellyfin/jellyfin:latest", 8096),
        a("Plex", "Media server", "Organize and stream your personal media collection anywhere.", "📺", "Media", "plexinc/pms-docker:latest", 32400),
        a("Nextcloud", "Files & collaboration", "Your own private cloud for files, calendars, contacts and more.", "☁️", "Productivity", "nextcloud:latest", 8080),
        a("Immich", "Photo backup", "High-performance self-hosted photo and video backup, an alternative to Google Photos.", "📷", "Media", "ghcr.io/immich-app/immich-server:release", 2283),
        a("Pi-hole", "Network ad-blocker", "Network-wide ad blocking via a DNS sinkhole.", "🛡️", "Network", "pihole/pihole:latest", 8081),
        a("Vaultwarden", "Password manager", "Lightweight Bitwarden-compatible password vault server.", "🔐", "Security", "vaultwarden/server:latest", 8082),
        a("Home Assistant", "Home automation", "Open-source home automation that puts local control and privacy first.", "🏠", "Automation", "ghcr.io/home-assistant/home-assistant:stable", 8123),
        a("qBittorrent", "Download client", "A free BitTorrent client with a clean web UI.", "🌐", "Downloads", "linuxserver/qbittorrent:latest", 8083),
        a("Grafana", "Dashboards", "Query, visualize and alert on your metrics.", "📊", "Monitoring", "grafana/grafana:latest", 3000),
        a("Portainer", "Container management", "A lightweight management UI for your container environment.", "🐳", "Management", "portainer/portainer-ce:latest", 9000),
        a("Paperless-ngx", "Document archive", "Scan, index and archive all your physical documents.", "📄", "Productivity", "ghcr.io/paperless-ngx/paperless-ngx:latest", 8084),
        a("AdGuard Home", "DNS filtering", "Network-wide software for blocking ads and tracking.", "🚫", "Network", "adguard/adguardhome:latest", 8085),
    ]
}

fn seed_apps(catalog: &[CatalogApp]) -> Vec<InstalledApp> {
    let mut rng = rand::thread_rng();
    let mut install = |cat: &CatalogApp, state: AppState, host_port: u16| InstalledApp {
        id: short_id("app"),
        catalog_id: cat.id.clone(),
        name: cat.name.clone(),
        icon: cat.icon.clone(),
        category: cat.category.clone(),
        image: cat.image.clone(),
        state,
        host_port,
        cpu_percent: if state == AppState::Running { rng.gen_range(0.5..12.0) } else { 0.0 },
        mem_bytes: if state == AppState::Running { rng.gen_range(80..900) * 1_000_000 } else { 0 },
        web_ui: Some(format!("http://ferrous-nas.local:{host_port}")),
        created_at: now_iso(),
    };
    let get = |id: &str| catalog.iter().find(|c| c.id == id).unwrap();
    vec![
        install(get("jellyfin"), AppState::Running, 8096),
        install(get("nextcloud"), AppState::Running, 8080),
        install(get("pi-hole"), AppState::Running, 8081),
        install(get("vaultwarden"), AppState::Stopped, 8082),
    ]
}

fn seed_users() -> (Vec<User>, Vec<Group>) {
    let admins = short_id("grp");
    let family = short_id("grp");
    let users = vec![
        User {
            id: short_id("user"),
            username: "gorav".to_string(),
            full_name: "Gorav (admin)".to_string(),
            is_admin: true,
            groups: vec!["admins".to_string()],
            created_at: now_iso(),
        },
        User {
            id: short_id("user"),
            username: "guest".to_string(),
            full_name: "Guest".to_string(),
            is_admin: false,
            groups: vec!["family".to_string()],
            created_at: now_iso(),
        },
    ];
    let groups = vec![
        Group { id: admins, name: "admins".to_string(), members: vec!["gorav".to_string()] },
        Group { id: family, name: "family".to_string(), members: vec!["guest".to_string()] },
    ];
    (users, groups)
}

fn seed_interfaces() -> Vec<NetInterface> {
    vec![
        NetInterface {
            name: "eth0".to_string(),
            mac: "de:ad:be:ef:00:01".to_string(),
            ipv4: Some("192.168.1.50/24".to_string()),
            ipv6: Some("fe80::dead:beef:1/64".to_string()),
            kind: "ethernet".to_string(),
            up: true,
            speed_mbps: 2500,
            rx_bytes: 84_213_998_112,
            tx_bytes: 22_909_113_004,
        },
        NetInterface {
            name: "eth1".to_string(),
            mac: "de:ad:be:ef:00:02".to_string(),
            ipv4: None,
            ipv6: None,
            kind: "ethernet".to_string(),
            up: false,
            speed_mbps: 1000,
            rx_bytes: 0,
            tx_bytes: 0,
        },
        NetInterface {
            name: "docker0".to_string(),
            mac: "02:42:ac:11:00:01".to_string(),
            ipv4: Some("172.17.0.1/16".to_string()),
            ipv6: None,
            kind: "bridge".to_string(),
            up: true,
            speed_mbps: 0,
            rx_bytes: 1_223_119,
            tx_bytes: 3_998_221,
        },
    ]
}

fn seed_alerts() -> Vec<Alert> {
    vec![
        Alert {
            id: short_id("alert"),
            level: AlertLevel::Warning,
            title: "S.M.A.R.T. warning on sdc".to_string(),
            message: "Disk sdc reported 3 reallocated sectors. Consider planning a replacement.".to_string(),
            created_at: now_iso(),
            acknowledged: false,
        },
        Alert {
            id: short_id("alert"),
            level: AlertLevel::Info,
            title: "Scrub completed on tank".to_string(),
            message: "Scheduled scrub of pool 'tank' finished with 0 errors.".to_string(),
            created_at: now_iso(),
            acknowledged: false,
        },
        Alert {
            id: short_id("alert"),
            level: AlertLevel::Info,
            title: "Update available".to_string(),
            message: "FerrousNAS 0.2.0 is available. Review the changelog before updating.".to_string(),
            created_at: now_iso(),
            acknowledged: true,
        },
    ]
}
