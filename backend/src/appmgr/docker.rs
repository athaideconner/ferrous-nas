//! Real app management via the Docker Engine API (unix socket).
//!
//! FerrousNAS-managed containers are tagged with `com.ferrousnas.*` labels and
//! named `ferrousnas-<catalog_id>`, so `list()` only ever sees containers this
//! app created — it never touches unrelated containers on the host.

use std::collections::HashMap;

use async_trait::async_trait;
use bollard::container::{
    Config, CreateContainerOptions, ListContainersOptions, RemoveContainerOptions,
    StartContainerOptions, StopContainerOptions,
};
use bollard::errors::Error as DockerError;
use bollard::image::CreateImageOptions;
use bollard::models::{ContainerSummary, HostConfig, PortBinding};
use bollard::Docker;
use futures_util::StreamExt;

use super::AppManager;
use crate::error::{ApiError, ApiResult};
use crate::models::*;

const LBL_MANAGED: &str = "com.ferrousnas.managed";
const LBL_CATALOG: &str = "com.ferrousnas.catalog_id";
const LBL_NAME: &str = "com.ferrousnas.name";
const LBL_ICON: &str = "com.ferrousnas.icon";
const LBL_CATEGORY: &str = "com.ferrousnas.category";
const LBL_PORT: &str = "com.ferrousnas.host_port";

pub struct DockerAppManager {
    docker: Docker,
}

impl DockerAppManager {
    /// Connect to the local Docker daemon and verify it responds.
    pub async fn connect() -> Result<Self, String> {
        let docker = Docker::connect_with_unix_defaults().map_err(|e| e.to_string())?;
        docker.ping().await.map_err(|e| e.to_string())?;
        Ok(Self { docker })
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
            // Live cpu/mem would need a per-container stats stream; left at 0
            // for now to keep listing cheap and robust.
            cpu_percent: 0.0,
            mem_bytes: 0,
            web_ui: if host_port > 0 {
                Some(format!("http://localhost:{host_port}"))
            } else {
                None
            },
            created_at: c
                .created
                .and_then(|s| chrono::DateTime::from_timestamp(s, 0))
                .map(|d| d.to_rfc3339())
                .unwrap_or_default(),
        }
    }

    async fn all(&self) -> ApiResult<Vec<InstalledApp>> {
        let mut filters = HashMap::new();
        filters.insert("label".to_string(), vec![format!("{LBL_MANAGED}=true")]);
        let opts = ListContainersOptions::<String> {
            all: true,
            filters,
            ..Default::default()
        };
        let list = self.docker.list_containers(Some(opts)).await.map_err(map_err)?;
        Ok(list.into_iter().map(Self::map_summary).collect())
    }

    async fn find(&self, id: &str) -> ApiResult<InstalledApp> {
        self.all()
            .await?
            .into_iter()
            .find(|a| a.id == id)
            .ok_or_else(|| ApiError::NotFound(format!("app {id} not found")))
    }
}

#[async_trait]
impl AppManager for DockerAppManager {
    fn source(&self) -> &'static str {
        "docker"
    }

    async fn list(&self) -> ApiResult<Vec<InstalledApp>> {
        self.all().await
    }

    async fn install(&self, cat: &CatalogApp, host_port: u16) -> ApiResult<InstalledApp> {
        if self.all().await?.iter().any(|a| a.catalog_id == cat.id) {
            return Err(ApiError::Conflict(format!("{} is already installed", cat.name)));
        }

        // Pull the image (consume the progress stream to completion).
        let (from_image, tag) = split_image(&cat.image);
        let mut pull = self.docker.create_image(
            Some(CreateImageOptions::<String> { from_image, tag, ..Default::default() }),
            None,
            None,
        );
        while let Some(step) = pull.next().await {
            step.map_err(|e| ApiError::BadRequest(format!("image pull failed: {e}")))?;
        }

        // Publish the app's port on the host.
        let port_key = format!("{}/tcp", cat.default_port);
        let mut exposed: HashMap<String, HashMap<(), ()>> = HashMap::new();
        exposed.insert(port_key.clone(), HashMap::new());
        let mut bindings: HashMap<String, Option<Vec<PortBinding>>> = HashMap::new();
        bindings.insert(
            port_key,
            Some(vec![PortBinding {
                host_ip: Some("0.0.0.0".to_string()),
                host_port: Some(host_port.to_string()),
            }]),
        );

        let mut labels: HashMap<String, String> = HashMap::new();
        labels.insert(LBL_MANAGED.into(), "true".into());
        labels.insert(LBL_CATALOG.into(), cat.id.clone());
        labels.insert(LBL_NAME.into(), cat.name.clone());
        labels.insert(LBL_ICON.into(), cat.icon.clone());
        labels.insert(LBL_CATEGORY.into(), cat.category.clone());
        labels.insert(LBL_PORT.into(), host_port.to_string());

        let config = Config {
            image: Some(cat.image.clone()),
            labels: Some(labels),
            exposed_ports: Some(exposed),
            host_config: Some(HostConfig {
                port_bindings: Some(bindings),
                ..Default::default()
            }),
            ..Default::default()
        };

        let name = format!("ferrousnas-{}", cat.id);
        let opts = CreateContainerOptions::<String> {
            name: name.clone(),
            ..Default::default()
        };
        let created = self.docker.create_container(Some(opts), config).await.map_err(map_err)?;
        self.docker
            .start_container(&created.id, None::<StartContainerOptions<String>>)
            .await
            .map_err(map_err)?;

        self.find(&created.id).await
    }

    async fn start(&self, id: &str) -> ApiResult<InstalledApp> {
        self.docker
            .start_container(id, None::<StartContainerOptions<String>>)
            .await
            .map_err(map_err)?;
        self.find(id).await
    }

    async fn stop(&self, id: &str) -> ApiResult<InstalledApp> {
        self.docker
            .stop_container(id, None::<StopContainerOptions>)
            .await
            .map_err(map_err)?;
        self.find(id).await
    }

    async fn uninstall(&self, id: &str) -> ApiResult<()> {
        self.docker
            .remove_container(
                id,
                Some(RemoveContainerOptions { force: true, v: true, link: false }),
            )
            .await
            .map_err(map_err)?;
        Ok(())
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
