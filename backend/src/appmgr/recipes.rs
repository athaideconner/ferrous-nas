//! How to actually run each catalog app under Docker.
//!
//! The catalog (`state::seed_catalog`) only describes an app for the store
//! page; a recipe is what makes the container work: the port its web UI
//! really listens on inside the container (often not the host port the
//! catalog picks), extra ports such as DNS, where its data lives so it
//! survives updates and reinstalls, settings it needs, and any helper
//! containers (databases, caches) it can't run without.
//!
//! String values may use these placeholders, filled in at install time:
//!
//! - `{port}` — the host port the app was installed on
//! - `{secret:NAME}` — a random password, generated on first install and
//!   reused on reinstall (the app's data volumes outlive an uninstall, and
//!   e.g. a database volume only accepts the password it was created with)
//! - `{c:ALIAS}` — the container name of this app's helper container ALIAS

/// Host directory for files people work with directly — media libraries,
/// downloads, scans — bind-mounted into the apps that use them, so they can
/// be shared over SMB/NFS like any other folder.
pub const FILES_ROOT: &str = "/srv/ferrous-nas";

pub struct Recipe {
    /// Port the web UI listens on inside the container; `None` means it
    /// listens on the host port itself (apps that must, see qBittorrent).
    pub web_port: Option<u16>,
    pub scheme: &'static str,
    /// Path the "Open" link goes to.
    pub path: &'static str,
    /// Run on the host's network instead of publishing ports: for apps that
    /// discover devices on the LAN or decide what's "local" by source IP.
    /// The app then always uses its own port, whatever host port is asked.
    pub host_network: bool,
    /// Ports published as-is (same number on host and container).
    pub extra_ports: &'static [(u16, Proto)],
    pub env: &'static [(&'static str, &'static str)],
    /// Named volumes: (name, path in the container).
    pub volumes: &'static [(&'static str, &'static str)],
    /// Folders under [`FILES_ROOT`]: (subdirectory, path in the container).
    pub files: &'static [(&'static str, &'static str)],
    pub docker_socket: bool,
    /// Serve HTTPS with the NAS's own certificate, mounted at
    /// /ssl/{cert,key}.pem, using this extra environment variable.
    pub tls_env: Option<(&'static str, &'static str)>,
    pub helpers: &'static [Helper],
    /// Shown to admins on the app's card: how to log in the first time.
    pub hint: Option<&'static str>,
    /// For apps that print their first password: the text right before it
    /// in the container log, and the hint to show around it (`{}`).
    pub log_hint: Option<(&'static str, &'static str)>,
}

#[derive(Clone, Copy)]
pub enum Proto {
    Tcp,
    Udp,
}

impl Proto {
    pub fn as_str(self) -> &'static str {
        match self {
            Proto::Tcp => "tcp",
            Proto::Udp => "udp",
        }
    }
}

/// A container the app needs alongside it, reachable from the app at the
/// container name `{c:ALIAS}` on the app's private network.
pub struct Helper {
    pub alias: &'static str,
    pub image: &'static str,
    pub env: &'static [(&'static str, &'static str)],
    pub volumes: &'static [(&'static str, &'static str)],
}

const BASE: Recipe = Recipe {
    web_port: None,
    scheme: "http",
    path: "/",
    host_network: false,
    extra_ports: &[],
    env: &[],
    volumes: &[],
    files: &[],
    docker_socket: false,
    tls_env: None,
    helpers: &[],
    hint: None,
    log_hint: None,
};

const DNS: &[(u16, Proto)] = &[(53, Proto::Tcp), (53, Proto::Udp)];

pub fn recipe(catalog_id: &str) -> Option<Recipe> {
    Some(match catalog_id {
        "jellyfin" => Recipe {
            web_port: Some(8096),
            volumes: &[("config", "/config"), ("cache", "/cache")],
            files: &[("media", "/media")],
            hint: Some("Create your account in the setup wizard. Media goes in /srv/ferrous-nas/media."),
            ..BASE
        },
        "plex" => Recipe {
            web_port: Some(32400),
            path: "/web",
            // Plex only allows first-time setup from what it sees as the
            // local network; behind Docker's NAT every request looks remote.
            host_network: true,
            volumes: &[("config", "/config"), ("transcode", "/transcode")],
            files: &[("media", "/data")],
            hint: Some("Sign in with your Plex account. Media goes in /srv/ferrous-nas/media."),
            ..BASE
        },
        "nextcloud" => Recipe {
            web_port: Some(80),
            volumes: &[("html", "/var/www/html")],
            hint: Some("Create the admin account on the first page."),
            ..BASE
        },
        "immich" => Recipe {
            web_port: Some(2283),
            env: &[
                ("DB_HOSTNAME", "{c:database}"),
                ("DB_USERNAME", "postgres"),
                ("DB_PASSWORD", "{secret:db}"),
                ("DB_DATABASE_NAME", "immich"),
                ("REDIS_HOSTNAME", "{c:redis}"),
                ("IMMICH_MACHINE_LEARNING_URL", "http://{c:ml}:3003"),
                ("IMMICH_MEDIA_LOCATION", "/data"),
            ],
            volumes: &[("upload", "/data")],
            helpers: &[
                Helper {
                    alias: "database",
                    image: "ghcr.io/immich-app/postgres:14-vectorchord0.4.3-pgvectors0.2.0",
                    env: &[
                        ("POSTGRES_USER", "postgres"),
                        ("POSTGRES_PASSWORD", "{secret:db}"),
                        ("POSTGRES_DB", "immich"),
                        ("POSTGRES_INITDB_ARGS", "--data-checksums"),
                    ],
                    volumes: &[("db", "/var/lib/postgresql/data")],
                },
                Helper { alias: "redis", image: "docker.io/valkey/valkey:8-bookworm", env: &[], volumes: &[] },
                Helper {
                    alias: "ml",
                    image: "ghcr.io/immich-app/immich-machine-learning:release",
                    env: &[],
                    volumes: &[("model-cache", "/cache")],
                },
            ],
            hint: Some("Create the admin account on the first page."),
            ..BASE
        },
        "pi-hole" => Recipe {
            web_port: Some(80),
            path: "/admin/",
            extra_ports: DNS,
            env: &[
                ("FTLCONF_webserver_api_password", "{secret:web}"),
                // Answer queries from the LAN, not just Docker's own network.
                ("FTLCONF_dns_listeningMode", "all"),
            ],
            volumes: &[("etc", "/etc/pihole")],
            hint: Some("Password: {secret:web} — then point your router's DNS at this NAS."),
            ..BASE
        },
        "vaultwarden" => Recipe {
            web_port: Some(80),
            volumes: &[("data", "/data")],
            // Its web vault refuses to run over plain HTTP.
            tls_env: Some(("ROCKET_TLS", r#"{certs="/ssl/cert.pem",key="/ssl/key.pem"}"#)),
            hint: Some("Create your account on the first page."),
            ..BASE
        },
        "home-assistant" => Recipe {
            web_port: Some(8123),
            // Finds devices on the LAN by multicast, which needs host networking.
            host_network: true,
            volumes: &[("config", "/config")],
            hint: Some("Create your account in the onboarding wizard."),
            ..BASE
        },
        "qbittorrent" => Recipe {
            // Rejects requests whose port differs from its own, so it has to
            // listen on the host port itself.
            web_port: None,
            extra_ports: &[(6881, Proto::Tcp), (6881, Proto::Udp)],
            env: &[
                ("WEBUI_PORT", "{port}"),
                ("TORRENTING_PORT", "6881"),
                // The first login account, so downloads aren't owned by an
                // id nobody can use.
                ("PUID", "1000"),
                ("PGID", "1000"),
            ],
            volumes: &[("config", "/config")],
            files: &[("media/downloads", "/downloads")],
            log_hint: Some((
                "temporary password is provided for this session: ",
                "admin / {} (temporary — set one under Tools → Options → WebUI)",
            )),
            ..BASE
        },
        "grafana" => Recipe {
            web_port: Some(3000),
            env: &[("GF_SECURITY_ADMIN_PASSWORD", "{secret:admin}")],
            volumes: &[("data", "/var/lib/grafana")],
            hint: Some("admin / {secret:admin}"),
            ..BASE
        },
        "portainer" => Recipe {
            web_port: Some(9000),
            docker_socket: true,
            volumes: &[("data", "/data")],
            hint: Some("Create the admin account within 5 minutes of starting it (or stop and start it again)."),
            ..BASE
        },
        "paperless-ngx" => Recipe {
            web_port: Some(8000),
            env: &[
                ("PAPERLESS_REDIS", "redis://{c:broker}:6379"),
                ("PAPERLESS_ADMIN_USER", "admin"),
                ("PAPERLESS_ADMIN_PASSWORD", "{secret:admin}"),
            ],
            volumes: &[
                ("data", "/usr/src/paperless/data"),
                ("media", "/usr/src/paperless/media"),
                ("export", "/usr/src/paperless/export"),
            ],
            files: &[("scans", "/usr/src/paperless/consume")],
            helpers: &[Helper { alias: "broker", image: "docker.io/library/redis:7", env: &[], volumes: &[("redis", "/data")] }],
            hint: Some("admin / {secret:admin} — drop scans in /srv/ferrous-nas/scans"),
            ..BASE
        },
        "adguard-home" => Recipe {
            // The setup wizard; it then offers to move the UI to port 80.
            web_port: Some(3000),
            extra_ports: DNS,
            volumes: &[("work", "/opt/adguardhome/work"), ("conf", "/opt/adguardhome/conf")],
            hint: Some("In the setup wizard, set the admin web interface port to 3000 so this link keeps working."),
            ..BASE
        },
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_catalog_app_has_a_recipe() {
        for app in &crate::state::Store::seeded().catalog {
            assert!(recipe(&app.id).is_some(), "no recipe for catalog app {}", app.id);
        }
    }

    #[test]
    fn helpers_are_only_referenced_if_they_exist() {
        for app in &crate::state::Store::seeded().catalog {
            let r = recipe(&app.id).unwrap();
            let aliases: Vec<_> = r.helpers.iter().map(|h| h.alias).collect();
            let all_env = r.env.iter().chain(r.helpers.iter().flat_map(|h| h.env.iter()));
            for (_, v) in all_env {
                for part in v.split("{c:").skip(1) {
                    let alias = part.split('}').next().unwrap();
                    assert!(aliases.contains(&alias), "{}: unknown helper {alias}", app.id);
                }
            }
        }
    }
}
