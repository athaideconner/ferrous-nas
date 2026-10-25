//! Real app management via the Docker Engine API (unix socket).
//!
//! FerrousNAS-managed containers are tagged with `com.ferrousnas.*` labels and
//! named `ferrousnas-<catalog_id>`, so `list()` only ever sees containers this
//! app created — it never touches unrelated containers on the host.
//!
//! How each app is run comes from its [recipe](super::recipes). An app with
//! helper containers (a database, a cache) gets its own network,
//! `ferrousnas-<catalog_id>`, and the helpers are named
//! `ferrousnas-<catalog_id>-<alias>` and labelled as belonging to the app, so
//! they start, stop and uninstall with it but aren't listed as apps.
//!
//! Data lives in named volumes (`ferrousnas-<catalog_id>-<name>`) that an
//! uninstall leaves in place, so reinstalling an app picks its data back up;
//! the passwords generated for it are kept for the same reason. Files people
//! work with directly (media, downloads, photos, scans) are instead folders
//! under the files root — the system disk by default, or a pool dataset the
//! admin picks — saved in `app-settings.json` in the state dir.

use std::collections::HashMap;
use std::path::PathBuf;

use async_trait::async_trait;
use bollard::container::{
    Config, CreateContainerOptions, ListContainersOptions, LogOutput, LogsOptions,
    RemoveContainerOptions, StartContainerOptions, Stats, StatsOptions, StopContainerOptions,
};
use bollard::errors::Error as DockerError;
use bollard::image::CreateImageOptions;
use bollard::models::{
    ContainerSummary, HostConfig, PortBinding, RestartPolicy, RestartPolicyNameEnum,
};
use bollard::network::CreateNetworkOptions;
use bollard::Docker;
use futures_util::future::join_all;
use futures_util::StreamExt;
use rand::distributions::{Alphanumeric, DistString};

use tokio::sync::RwLock;

use super::recipes::{self, Recipe, DEFAULT_FILES_ROOT};
use super::AppManager;
use crate::error::{ApiError, ApiResult};
use crate::models::*;

const LBL_MANAGED: &str = "com.ferrousnas.managed";
const LBL_CATALOG: &str = "com.ferrousnas.catalog_id";
const LBL_NAME: &str = "com.ferrousnas.name";
const LBL_ICON: &str = "com.ferrousnas.icon";
const LBL_CATEGORY: &str = "com.ferrousnas.category";
const LBL_PORT: &str = "com.ferrousnas.host_port";
const LBL_WEB: &str = "com.ferrousnas.web_ui";
const LBL_HINT: &str = "com.ferrousnas.hint";
/// On helper containers: the catalog id of the app they belong to.
const LBL_HELPER_OF: &str = "com.ferrousnas.helper_of";

pub struct DockerAppManager {
    docker: Docker,
    files_root: RwLock<String>,
}

impl DockerAppManager {
    /// Connect to the local Docker daemon and verify it responds.
    pub async fn connect() -> Result<Self, String> {
        let docker = Docker::connect_with_unix_defaults().map_err(|e| e.to_string())?;
        docker.ping().await.map_err(|e| e.to_string())?;
        Ok(Self { docker, files_root: RwLock::new(load_files_root()) })
    }

    fn map_summary(c: ContainerSummary) -> InstalledApp {
        let labels = c.labels.unwrap_or_default();
        let get = |k: &str| labels.get(k).cloned();
        let state = match c.state.as_deref() {
            Some("running") => AppState::Running,
            Some("created") | Some("exited") | Some("paused") | Some("dead") => AppState::Stopped,
            _ => AppState::Error,
        };
        let host_port: u16 = get(LBL_PORT).and_then(|v| v.parse().ok()).unwrap_or(0);
        let image = c.image.unwrap_or_default();
        InstalledApp {
            id: c.id.unwrap_or_default(),
            catalog_id: get(LBL_CATALOG).unwrap_or_default(),
            name: get(LBL_NAME).unwrap_or_else(|| image.clone()),
            icon: get(LBL_ICON).unwrap_or_else(|| "📦".into()),
            category: get(LBL_CATEGORY).unwrap_or_else(|| "Other".into()),
            image,
            state,
            host_port,
            cpu_percent: 0.0,
            mem_bytes: 0,
            web_ui: get(LBL_WEB).or_else(|| {
                (host_port > 0).then(|| format!("http://{{host}}:{host_port}/"))
            }),
            hint: get(LBL_HINT),
            created_at: c
                .created
                .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
                .map(|d| d.to_rfc3339())
                .unwrap_or_default(),
        }
    }

    async fn containers(&self, label: String) -> ApiResult<Vec<ContainerSummary>> {
        let mut filters = HashMap::new();
        filters.insert("label".to_string(), vec![label]);
        let opts = ListContainersOptions::<String> { all: true, filters, ..Default::default() };
        self.docker.list_containers(Some(opts)).await.map_err(map_err)
    }

    async fn all(&self) -> ApiResult<Vec<InstalledApp>> {
        Ok(self
            .containers(format!("{LBL_MANAGED}=true"))
            .await?
            .into_iter()
            .map(Self::map_summary)
            .collect())
    }

    async fn find(&self, id: &str) -> ApiResult<InstalledApp> {
        self.all()
            .await?
            .into_iter()
            .find(|a| a.id == id)
            .ok_or_else(|| ApiError::NotFound(format!("app {id} not found")))
    }

    /// Ids of the helper containers belonging to `catalog_id`.
    async fn helpers(&self, catalog_id: &str) -> ApiResult<Vec<String>> {
        Ok(self
            .containers(format!("{LBL_HELPER_OF}={catalog_id}"))
            .await?
            .into_iter()
            .filter_map(|c| c.id)
            .collect())
    }

    async fn pull(&self, image: &str) -> ApiResult<()> {
        let (from_image, tag) = split_image(image);
        let mut pull = self.docker.create_image(
            Some(CreateImageOptions::<String> { from_image, tag, ..Default::default() }),
            None,
            None,
        );
        while let Some(step) = pull.next().await {
            step.map_err(|e| ApiError::BadRequest(format!("pulling {image} failed: {e}")))?;
        }
        Ok(())
    }

    async fn create_and_start(&self, name: &str, config: Config<String>) -> ApiResult<String> {
        let opts = CreateContainerOptions::<String> { name: name.to_string(), ..Default::default() };
        let created = self.docker.create_container(Some(opts), config).await.map_err(map_err)?;
        self.docker
            .start_container(&created.id, None::<StartContainerOptions<String>>)
            .await
            .map_err(map_err)?;
        Ok(created.id)
    }

    /// Remove an app's containers and network (best effort; volumes stay).
    async fn remove_all(&self, catalog_id: &str) {
        let name = format!("ferrousnas-{catalog_id}");
        let mut ids = self.helpers(catalog_id).await.unwrap_or_default();
        ids.push(name.clone());
        for id in ids {
            let _ = self
                .docker
                .remove_container(&id, Some(RemoveContainerOptions { force: true, v: false, link: false }))
                .await;
        }
        let _ = self.docker.remove_network(&name).await;
    }

    async fn do_install(&self, cat: &CatalogApp, r: &Recipe, host_port: u16) -> ApiResult<InstalledApp> {
        let name = format!("ferrousnas-{}", cat.id);
        let host_port = match (r.host_network, r.web_port) {
            (true, Some(p)) => p,
            _ => host_port,
        };
        let secrets = load_secrets(&cat.id);
        let files_root = self.files_root.read().await.clone();
        let subst = |s: &str| fill(s, &cat.id, host_port, &files_root, &secrets);

        // Helpers first, on the app's own network, so they're up (or at
        // least starting) by the time the app looks for them.
        if !r.helpers.is_empty() {
            let mut labels = HashMap::new();
            labels.insert(LBL_HELPER_OF.to_string(), cat.id.clone());
            let opts = CreateNetworkOptions::<String> { name: name.clone(), labels, ..Default::default() };
            self.docker.create_network(opts).await.map_err(map_err)?;
        }
        for h in r.helpers {
            self.pull(h.image).await?;
            let mut labels = HashMap::new();
            labels.insert(LBL_HELPER_OF.to_string(), cat.id.clone());
            let config = Config {
                image: Some(h.image.to_string()),
                env: Some(h.env.iter().map(|(k, v)| format!("{k}={}", subst(v))).collect()),
                labels: Some(labels),
                host_config: Some(HostConfig {
                    binds: Some(volume_binds(&cat.id, h.volumes)),
                    network_mode: Some(name.clone()),
                    restart_policy: Some(unless_stopped()),
                    ..Default::default()
                }),
                ..Default::default()
            };
            self.create_and_start(&format!("{name}-{}", h.alias), config).await?;
        }

        self.pull(&cat.image).await?;

        let mut env: Vec<String> = r.env.iter().map(|(k, v)| format!("{k}={}", subst(v))).collect();
        let mut binds = volume_binds(&cat.id, r.volumes);
        for (dir, path) in r.files {
            let host = format!("{files_root}/{dir}");
            // Made here, not left to Docker, which would create it owned by
            // root and unwritable for apps running as a user.
            super::prepare_files_dir(&files_root, dir)
                .map_err(|e| ApiError::BadRequest(format!("preparing {host}: {e}")))?;
            binds.push(format!("{host}:{path}"));
        }
        if r.docker_socket {
            binds.push("/var/run/docker.sock:/var/run/docker.sock".into());
        }
        let mut scheme = r.scheme;
        if let Some((k, v)) = r.tls_env {
            // Only if the daemon's own self-signed certificate is there to
            // reuse; with TLS off or a custom certificate the app stays HTTP.
            let (cert, key) = (state_dir().join("tls-cert.pem"), state_dir().join("tls-key.pem"));
            if cert.exists() && key.exists() {
                binds.push(format!("{}:/ssl/cert.pem:ro", cert.display()));
                binds.push(format!("{}:/ssl/key.pem:ro", key.display()));
                env.push(format!("{k}={v}"));
                scheme = "https";
            }
        }

        // Publish the web UI and any extra ports, unless on the host network.
        let web_port = r.web_port.unwrap_or(host_port);
        let mut ports = vec![(host_port, web_port, "tcp")];
        ports.extend(r.extra_ports.iter().map(|(p, proto)| (*p, *p, proto.as_str())));
        let mut exposed: HashMap<String, HashMap<(), ()>> = HashMap::new();
        let mut bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
        if !r.host_network {
            for (host, container, proto) in ports {
                let key = format!("{container}/{proto}");
                exposed.insert(key.clone(), HashMap::new());
                bindings.insert(
                    key,
                    Some(vec![PortBinding {
                        host_ip: Some("0.0.0.0".to_string()),
                        host_port: Some(host.to_string()),
                    }]),
                );
            }
        }

        let mut labels: HashMap<String, String> = HashMap::new();
        labels.insert(LBL_MANAGED.into(), "true".into());
        labels.insert(LBL_CATALOG.into(), cat.id.clone());
        labels.insert(LBL_NAME.into(), cat.name.clone());
        labels.insert(LBL_ICON.into(), cat.icon.clone());
        labels.insert(LBL_CATEGORY.into(), cat.category.clone());
        labels.insert(LBL_PORT.into(), host_port.to_string());
        // `{host}` is filled in by the browser: whatever address it reached
        // the dashboard on also reaches the app.
        labels.insert(LBL_WEB.into(), format!("{scheme}://{{host}}:{host_port}{}", r.path));
        if let Some(hint) = r.hint {
            labels.insert(LBL_HINT.into(), subst(hint));
        }

        let network_mode = if r.host_network {
            Some("host".to_string())
        } else if !r.helpers.is_empty() {
            Some(name.clone())
        } else {
            None
        };
        let config = Config {
            image: Some(cat.image.clone()),
            env: Some(env),
            labels: Some(labels),
            exposed_ports: Some(exposed),
            host_config: Some(HostConfig {
                port_bindings: Some(bindings),
                binds: Some(binds),
                network_mode,
                restart_policy: Some(unless_stopped()),
                ..Default::default()
            }),
            ..Default::default()
        };
        let id = self.create_and_start(&name, config).await?;
        self.find(&id).await
    }

    /// CPU % and memory of a running container, summed with its helpers'.
    async fn usage(&self, catalog_id: &str, id: &str) -> (f32, u64) {
        let mut ids = self.helpers(catalog_id).await.unwrap_or_default();
        ids.push(id.to_string());
        let stats = join_all(ids.iter().map(|id| self.stats(id))).await;
        stats.into_iter().flatten().fold((0.0, 0), |(c, m), (c2, m2)| (c + c2, m + m2))
    }

    async fn stats(&self, id: &str) -> Option<(f32, u64)> {
        // Not one-shot: the second sample is what CPU % is measured against.
        let opts = StatsOptions { stream: false, one_shot: false };
        let s: Stats = self.docker.stats(id, Some(opts)).next().await?.ok()?;
        let cpu_delta = s.cpu_stats.cpu_usage.total_usage.saturating_sub(s.precpu_stats.cpu_usage.total_usage);
        let sys_delta = s
            .cpu_stats
            .system_cpu_usage
            .unwrap_or(0)
            .saturating_sub(s.precpu_stats.system_cpu_usage.unwrap_or(0));
        let cpus = s.cpu_stats.online_cpus.unwrap_or(1) as f32;
        let cpu = if sys_delta > 0 { cpu_delta as f32 / sys_delta as f32 * cpus * 100.0 } else { 0.0 };
        // Like `docker stats`: usage minus reclaimable page cache.
        let cache = match s.memory_stats.stats {
            Some(bollard::container::MemoryStatsStats::V2(v2)) => v2.inactive_file,
            Some(bollard::container::MemoryStatsStats::V1(v1)) => v1.total_inactive_file,
            None => 0,
        };
        Some((cpu, s.memory_stats.usage.unwrap_or(0).saturating_sub(cache)))
    }

    /// For apps that print their first password: find it in the log.
    async fn log_hint(&self, id: &str, (marker, template): (&str, &str)) -> Option<String> {
        let opts = LogsOptions::<String> { stdout: true, stderr: true, tail: "500".into(), ..Default::default() };
        let mut logs = self.docker.logs(id, Some(opts));
        let mut found = None;
        while let Some(Ok(chunk)) = logs.next().await {
            let text = match chunk {
                LogOutput::StdOut { message } | LogOutput::StdErr { message } | LogOutput::Console { message } => message,
                _ => continue,
            };
            let text = String::from_utf8_lossy(&text);
            for line in text.lines() {
                if let Some(rest) = line.split_once(marker).map(|(_, r)| r) {
                    if let Some(pw) = rest.split_whitespace().next() {
                        found = Some(template.replace("{}", pw));
                    }
                }
            }
        }
        found
    }
}

#[async_trait]
impl AppManager for DockerAppManager {
    fn source(&self) -> &'static str {
        "docker"
    }

    async fn list(&self) -> ApiResult<Vec<InstalledApp>> {
        let apps = self.all().await?;
        let filled = apps.into_iter().map(|mut a| async move {
            if a.state == AppState::Running {
                (a.cpu_percent, a.mem_bytes) = self.usage(&a.catalog_id, &a.id).await;
                if let Some(lh) = recipes::recipe(&a.catalog_id).and_then(|r| r.log_hint) {
                    a.hint = self.log_hint(&a.id, lh).await.or(a.hint);
                }
            }
            a
        });
        Ok(join_all(filled).await)
    }

    async fn install(&self, cat: &CatalogApp, host_port: u16) -> ApiResult<InstalledApp> {
        if self.all().await?.iter().any(|a| a.catalog_id == cat.id) {
            return Err(ApiError::Conflict(format!("{} is already installed", cat.name)));
        }
        let r = recipes::recipe(&cat.id)
            .ok_or_else(|| ApiError::BadRequest(format!("{} can't be installed: no recipe for it", cat.name)))?;

        // Leftovers of an earlier failed attempt would block the names.
        self.remove_all(&cat.id).await;
        match self.do_install(cat, &r, host_port).await {
            Ok(app) => Ok(app),
            Err(e) => {
                self.remove_all(&cat.id).await;
                Err(explain_port_clash(e))
            }
        }
    }

    async fn start(&self, id: &str) -> ApiResult<InstalledApp> {
        let app = self.find(id).await?;
        for h in self.helpers(&app.catalog_id).await? {
            self.docker.start_container(&h, None::<StartContainerOptions<String>>).await.or_else(already)?;
        }
        self.docker.start_container(id, None::<StartContainerOptions<String>>).await.or_else(already)?;
        self.find(id).await
    }

    async fn stop(&self, id: &str) -> ApiResult<InstalledApp> {
        let app = self.find(id).await?;
        self.docker.stop_container(id, None::<StopContainerOptions>).await.or_else(already)?;
        for h in self.helpers(&app.catalog_id).await? {
            self.docker.stop_container(&h, None::<StopContainerOptions>).await.or_else(already)?;
        }
        self.find(id).await
    }

    async fn uninstall(&self, id: &str) -> ApiResult<()> {
        let app = self.find(id).await?;
        self.remove_all(&app.catalog_id).await;
        Ok(())
    }

    async fn files_root(&self) -> String {
        self.files_root.read().await.clone()
    }

    async fn set_files_root(&self, root: &str, catalog: &[CatalogApp]) -> ApiResult<()> {
        let json = serde_json::json!({ "files_root": root }).to_string();
        std::fs::write(state_dir().join("app-settings.json"), json)
            .map_err(|e| ApiError::BadRequest(format!("saving the files location: {e}")))?;
        *self.files_root.write().await = root.to_string();

        // Recreate the installed apps that keep files there, so they use the
        // new location. Their volumes (settings, databases) stay as they are;
        // files already in the old location are not moved.
        let mut failed = Vec::new();
        for app in self.all().await? {
            let (Some(r), Some(cat)) =
                (recipes::recipe(&app.catalog_id), catalog.iter().find(|c| c.id == app.catalog_id))
            else {
                continue;
            };
            if r.files.is_empty() {
                continue;
            }
            self.remove_all(&cat.id).await;
            match self.do_install(cat, &r, app.host_port).await {
                Ok(new) if app.state != AppState::Running => {
                    let _ = self.docker.stop_container(&new.id, None::<StopContainerOptions>).await;
                }
                Ok(_) => {}
                Err(e) => failed.push(format!("{} ({e})", app.name)),
            }
        }
        if failed.is_empty() {
            Ok(())
        } else {
            Err(ApiError::BadRequest(format!(
                "Saved the new location, but couldn't move {}: reinstall them from the store.",
                failed.join(", ")
            )))
        }
    }
}

/// Docker's "Bind for 0.0.0.0:53 failed: port is already allocated", as
/// something a person can act on.
fn explain_port_clash(e: ApiError) -> ApiError {
    let msg = e.to_string();
    if !(msg.contains("port is already allocated") || msg.contains("address already in use")) {
        return e;
    }
    let port = msg
        .split("0.0.0.0:")
        .nth(1)
        .map(|r| r.chars().take_while(char::is_ascii_digit).collect::<String>())
        .filter(|p| !p.is_empty());
    ApiError::Conflict(match port {
        Some(p) => format!("Port {p} is already in use, probably by another installed app (Pi-hole and AdGuard Home both need port 53). Uninstall that app first."),
        None => format!("A port this app needs is already in use: {msg}"),
    })
}

fn unless_stopped() -> RestartPolicy {
    RestartPolicy { name: Some(RestartPolicyNameEnum::UNLESS_STOPPED), maximum_retry_count: None }
}

/// "Already started/stopped" (HTTP 304) is success for start/stop.
fn already(e: DockerError) -> ApiResult<()> {
    match e {
        DockerError::DockerResponseServerError { status_code: 304, .. } => Ok(()),
        e => Err(map_err(e)),
    }
}

fn volume_binds(catalog_id: &str, volumes: &[(&str, &str)]) -> Vec<String> {
    volumes.iter().map(|(v, path)| format!("ferrousnas-{catalog_id}-{v}:{path}")).collect()
}

/// Fill a recipe string's placeholders (see the `recipes` module docs).
fn fill(s: &str, catalog_id: &str, port: u16, files_root: &str, secrets: &HashMap<String, String>) -> String {
    let mut out = s.replace("{port}", &port.to_string()).replace("{files}", files_root);
    while let Some(start) = out.find("{c:") {
        let Some(len) = out[start..].find('}') else { break };
        let alias = out[start + 3..start + len].to_string();
        out.replace_range(start..=start + len, &format!("ferrousnas-{catalog_id}-{alias}"));
    }
    while let Some(start) = out.find("{secret:") {
        let Some(len) = out[start..].find('}') else { break };
        let key = &out[start + 8..start + len];
        let value = secrets.get(key).cloned().unwrap_or_default();
        out.replace_range(start..=start + len, &value);
    }
    out
}

/// Where app files live: as last chosen (app-settings.json), else the
/// system-disk default.
fn load_files_root() -> String {
    std::fs::read_to_string(state_dir().join("app-settings.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<serde_json::Value>(&s).ok())
        .and_then(|v| v.get("files_root")?.as_str().map(str::to_string))
        .unwrap_or_else(|| DEFAULT_FILES_ROOT.to_string())
}

fn state_dir() -> PathBuf {
    PathBuf::from(std::env::var("FERROUS_STATE_DIR").unwrap_or_else(|_| "/var/lib/ferrous-nas".into()))
}

/// The app's generated passwords, creating (and saving) any its recipe
/// needs that don't exist yet. Kept in the state dir, so a reinstall gets
/// the same ones its surviving data volumes were set up with.
fn load_secrets(catalog_id: &str) -> HashMap<String, String> {
    let path = state_dir().join("app-secrets.json");
    let mut all: HashMap<String, HashMap<String, String>> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    let Some(r) = recipes::recipe(catalog_id) else { return HashMap::new() };
    let texts = r
        .env
        .iter()
        .chain(r.helpers.iter().flat_map(|h| h.env.iter()))
        .map(|(_, v)| *v)
        .chain(r.hint);
    let entry = all.entry(catalog_id.to_string()).or_default();
    let mut changed = false;
    for text in texts {
        for part in text.split("{secret:").skip(1) {
            let key = part.split('}').next().unwrap_or_default().to_string();
            entry.entry(key).or_insert_with(|| {
                changed = true;
                Alphanumeric.sample_string(&mut rand::thread_rng(), 20)
            });
        }
    }
    let secrets = entry.clone();
    if changed {
        if let Ok(json) = serde_json::to_string_pretty(&all) {
            write_private(&path, &json);
        }
    }
    secrets
}

fn write_private(path: &std::path::Path, contents: &str) {
    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let result = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(path)
        .and_then(|mut f| f.write_all(contents.as_bytes()));
    if let Err(e) = result {
        tracing::warn!("apps: couldn't save generated passwords to {}: {e}", path.display());
    }
}

/// Split "repo/name:tag" into (from_image, tag), defaulting to "latest".
/// Careful not to treat a registry port (host:5000/img) as a tag.
fn split_image(image: &str) -> (String, String) {
    match image.rsplit_once(':') {
        Some((img, tag)) if !tag.contains('/') => (img.to_string(), tag.to_string()),
        _ => (image.to_string(), "latest".to_string()),
    }
}

fn map_err(e: DockerError) -> ApiError {
    match e {
        DockerError::DockerResponseServerError { status_code: 404, message } => ApiError::NotFound(message),
        DockerError::DockerResponseServerError { status_code: 409, message } => ApiError::Conflict(message),
        other => ApiError::BadRequest(other.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_replaces_every_placeholder() {
        let mut secrets = HashMap::new();
        secrets.insert("db".to_string(), "s3cret".to_string());
        assert_eq!(
            fill("postgres://{c:database}:{port}/x?pw={secret:db}&again={secret:db} {files}", "immich", 2283, "/tank/files", &secrets),
            "postgres://ferrousnas-immich-database:2283/x?pw=s3cret&again=s3cret /tank/files"
        );
    }
}
