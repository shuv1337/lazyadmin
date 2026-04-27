#![forbid(unsafe_code)]
#![deny(missing_debug_implementations)]
use async_trait::async_trait;
use bollard::{Docker, container::ListContainersOptions};
use chrono::Utc;
use futures::stream::BoxStream;
use lazyadmin_core::{
    graph::{
        AdapterCapabilities, AdapterHealth, DiscoveryAdapter, DiscoveryContext, DiscoveryOutput,
    },
    model::*,
};
use serde::Deserialize;
use std::{collections::HashMap, env, path::PathBuf, time::Instant};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ContainerRuntimeKind {
    Docker,
    PodmanRootless,
    PodmanRootful,
    UnknownDockerCompatible,
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApiFlavor {
    DockerCompatible,
    PodmanCompatible,
    LibpodSpecific,
}
#[derive(Clone, Debug)]
pub struct RuntimeEndpoint {
    pub source: String,
    pub uri: String,
    pub socket: Option<PathBuf>,
    pub kind: ContainerRuntimeKind,
    pub flavor: ApiFlavor,
}
#[derive(Clone, Debug, Default)]
pub struct ContainerAdapter {
    endpoints: Vec<RuntimeEndpoint>,
}
#[derive(thiserror::Error, Debug)]
pub enum ContainerError {
    #[error(transparent)]
    Bollard(#[from] bollard::errors::Error),
}

fn prov(claim: &str, evidence: impl Into<String>, confidence: Confidence) -> Provenance {
    Provenance {
        adapter: "container".into(),
        claim: claim.into(),
        evidence: evidence.into(),
        confidence,
        timestamp: Utc::now(),
    }
}
fn edge(kind: EdgeKind, from: EntityRef, to: EntityRef, evidence: impl Into<String>) -> Edge {
    Edge {
        kind,
        from,
        to,
        provenance: vec![prov("container correlation", evidence, Confidence::High)],
    }
}
fn warn(code: &str, msg: impl Into<String>, entity: Option<EntityRef>) -> Warning {
    Warning {
        severity: WarningSeverity::Warning,
        code: code.into(),
        message: msg.into(),
        entity,
        provenance: vec![prov("warning", code, Confidence::High)],
    }
}

impl ContainerAdapter {
    pub fn new() -> Self {
        Self {
            endpoints: default_endpoints(),
        }
    }
    pub fn endpoints(&self) -> &[RuntimeEndpoint] {
        &self.endpoints
    }
}
impl Default for RuntimeEndpoint {
    fn default() -> Self {
        Self {
            source: "default".into(),
            uri: "unix:///var/run/docker.sock".into(),
            socket: Some(PathBuf::from("/var/run/docker.sock")),
            kind: ContainerRuntimeKind::Docker,
            flavor: ApiFlavor::DockerCompatible,
        }
    }
}
pub fn default_endpoints() -> Vec<RuntimeEndpoint> {
    let mut v = Vec::new();
    if let Ok(h) = env::var("DOCKER_HOST") {
        v.push(RuntimeEndpoint {
            source: "$DOCKER_HOST".into(),
            uri: h,
            socket: None,
            kind: ContainerRuntimeKind::UnknownDockerCompatible,
            flavor: ApiFlavor::DockerCompatible,
        });
    }
    v.push(RuntimeEndpoint {
        source: "/var/run/docker.sock".into(),
        uri: "unix:///var/run/docker.sock".into(),
        socket: Some("/var/run/docker.sock".into()),
        kind: ContainerRuntimeKind::Docker,
        flavor: ApiFlavor::DockerCompatible,
    });
    v.push(RuntimeEndpoint {
        source: "/run/podman/podman.sock".into(),
        uri: "unix:///run/podman/podman.sock".into(),
        socket: Some("/run/podman/podman.sock".into()),
        kind: ContainerRuntimeKind::PodmanRootful,
        flavor: ApiFlavor::PodmanCompatible,
    });
    if let Ok(xdg) = env::var("XDG_RUNTIME_DIR") {
        let p = PathBuf::from(xdg).join("podman/podman.sock");
        v.push(RuntimeEndpoint {
            source: "$XDG_RUNTIME_DIR/podman/podman.sock".into(),
            uri: format!("unix://{}", p.display()),
            socket: Some(p),
            kind: ContainerRuntimeKind::PodmanRootless,
            flavor: ApiFlavor::PodmanCompatible,
        });
    }
    v
}
fn runtime_kind(k: &ContainerRuntimeKind, compose: bool) -> RuntimeKind {
    match (k, compose) {
        (ContainerRuntimeKind::PodmanRootful | ContainerRuntimeKind::PodmanRootless, true) => {
            RuntimeKind::PodmanCompose
        }
        (_, true) => RuntimeKind::DockerCompose,
        (ContainerRuntimeKind::PodmanRootful | ContainerRuntimeKind::PodmanRootless, false) => {
            RuntimeKind::Podman
        }
        _ => RuntimeKind::Docker,
    }
}
fn manager_id(ep: &RuntimeEndpoint) -> ManagerId {
    ManagerId::new(format!("container:{}", ep.source))
}
fn docker_from_endpoint(ep: &RuntimeEndpoint) -> Result<Docker, bollard::errors::Error> {
    if ep.uri.starts_with("unix://") {
        Docker::connect_with_unix(
            ep.uri.trim_start_matches("unix://"),
            120,
            bollard::API_DEFAULT_VERSION,
        )
    } else {
        Docker::connect_with_http(&ep.uri, 120, bollard::API_DEFAULT_VERSION)
    }
}
fn classify(ep: &RuntimeEndpoint, text: &str) -> ContainerRuntimeKind {
    let t = text.to_ascii_lowercase();
    if t.contains("podman") {
        ep.kind.clone()
    } else if t.contains("docker") {
        ContainerRuntimeKind::Docker
    } else {
        ep.kind.clone()
    }
}

#[async_trait]
impl DiscoveryAdapter for ContainerAdapter {
    fn name(&self) -> &'static str {
        "container"
    }
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            polling: true,
            watching: false,
        }
    }
    async fn health(&self) -> AdapterHealth {
        AdapterHealth {
            adapter: "container".into(),
            available: self.endpoints.iter().any(|e| {
                e.socket.as_ref().is_some_and(|p| p.exists()) || e.source == "$DOCKER_HOST"
            }),
            message: Some(format!(
                "{} endpoints configured; podman read-only",
                self.endpoints.len()
            )),
        }
    }
    #[tracing::instrument(name = "adapter.container.probe", skip_all)]
    async fn discover(&self, _: DiscoveryContext) -> anyhow::Result<DiscoveryOutput> {
        let mut out = DiscoveryOutput::default();
        for ep in &self.endpoints {
            let start = Instant::now();
            let mut manager = Manager {
                id: manager_id(ep),
                kind: runtime_kind(&ep.kind, false),
                name: ep.source.clone(),
                scope: ManagerScope::Container,
                socket: ep.socket.clone(),
                available: false,
                permission: PermissionState::Unknown,
                version: None,
                provenance: vec![prov("configured endpoint", &ep.uri, Confidence::Medium)],
            };
            if ep.socket.as_ref().is_some_and(|p| !p.exists()) {
                out.managers.push(manager);
                continue;
            }
            match docker_from_endpoint(ep) {
                Ok(d) => match d.version().await {
                    Ok(ver) => {
                        let raw = format!("{:?}", ver);
                        let ck = classify(ep, &raw);
                        manager.kind = runtime_kind(&ck, false);
                        manager.available = true;
                        manager.permission = PermissionState::Ok;
                        manager.version = ver.version;
                        manager
                            .provenance
                            .push(prov("version/info", raw, Confidence::High));
                        let opts = Some(ListContainersOptions::<String> {
                            all: false,
                            ..Default::default()
                        });
                        if let Ok(list) = d.list_containers(opts).await {
                            let json = serde_json::to_value(&list).unwrap_or_default();
                            let part =
                                discover_from_docker_list_value(&json, &ck, manager.id.clone());
                            out.merge(part);
                        }
                    }
                    Err(e) => {
                        let msg = e.to_string();
                        manager.permission =
                            if msg.contains("Permission") || msg.contains("permission") {
                                PermissionState::Denied
                            } else {
                                PermissionState::Unknown
                            };
                        manager
                            .provenance
                            .push(prov("probe failed", msg, Confidence::Medium));
                    }
                },
                Err(e) => {
                    let msg = e.to_string();
                    manager.permission = if msg.contains("Permission") || msg.contains("permission")
                    {
                        PermissionState::Denied
                    } else {
                        PermissionState::Unknown
                    };
                    manager
                        .provenance
                        .push(prov("connect failed", msg, Confidence::Medium));
                }
            }
            tracing::debug!(source=%ep.source, reachable=manager.available, permission_state=?manager.permission, duration_ms=start.elapsed().as_millis(), "container probe");
            out.managers.push(manager);
        }
        Ok(out)
    }

    #[tracing::instrument(name = "adapter.watch.start", skip_all, fields(adapter = "container"))]
    async fn watch(&self) -> Option<BoxStream<'static, DiscoveryEvent>> {
        None
    }
}

trait MergeOut {
    fn merge(&mut self, other: DiscoveryOutput);
}
impl MergeOut for DiscoveryOutput {
    fn merge(&mut self, o: DiscoveryOutput) {
        self.managers.extend(o.managers);
        self.processes.extend(o.processes);
        self.listeners.extend(o.listeners);
        self.workloads.extend(o.workloads);
        self.projects.extend(o.projects);
        self.tracked_runs.extend(o.tracked_runs);
        self.edges.extend(o.edges);
        self.warnings.extend(o.warnings);
    }
}

#[derive(Deserialize, Debug)]
struct RawPort {
    #[serde(rename = "IP")]
    ip: Option<String>,
    #[serde(rename = "PrivatePort")]
    private_port: u16,
    #[serde(rename = "PublicPort")]
    public_port: Option<u16>,
    #[serde(rename = "Type")]
    typ: Option<String>,
}
#[derive(Deserialize, Debug)]
struct RawContainer {
    #[serde(rename = "Id")]
    id: String,
    #[serde(rename = "Names", default)]
    names: Vec<String>,
    #[serde(rename = "Image", default)]
    image: String,
    #[serde(rename = "State", default)]
    state: String,
    #[serde(rename = "Status", default)]
    status: String,
    #[serde(rename = "Labels", default)]
    labels: HashMap<String, String>,
    #[serde(rename = "Ports", default)]
    ports: Vec<RawPort>,
}

pub fn discover_from_docker_list_json(
    s: &str,
    kind: &ContainerRuntimeKind,
    manager: ManagerId,
) -> anyhow::Result<DiscoveryOutput> {
    let v: serde_json::Value = serde_json::from_str(s)?;
    Ok(discover_from_docker_list_value(&v, kind, manager))
}
fn discover_from_docker_list_value(
    v: &serde_json::Value,
    kind: &ContainerRuntimeKind,
    manager: ManagerId,
) -> DiscoveryOutput {
    let mut out = DiscoveryOutput::default();
    let arr = if v.is_array() {
        v.clone()
    } else {
        v.get("containers")
            .cloned()
            .unwrap_or_else(|| serde_json::Value::Array(vec![]))
    };
    let Ok(containers) = serde_json::from_value::<Vec<RawContainer>>(arr) else {
        return out;
    };
    for c in containers {
        let dproj = c
            .labels
            .get("com.docker.compose.project")
            .or_else(|| c.labels.get("io.podman.compose.project"));
        let dsvc = c
            .labels
            .get("com.docker.compose.service")
            .or_else(|| c.labels.get("io.podman.compose.service"));
        let compose = dproj.zip(dsvc);
        let wid = if let Some((p, s)) = compose {
            WorkloadId::new(format!("compose:{p}/{s}"))
        } else {
            WorkloadId::new(format!("container:{}", &c.id[..12.min(c.id.len())]))
        };
        let name = compose.map(|(p, s)| format!("{p}/{s}")).unwrap_or_else(|| {
            c.names
                .first()
                .map(|n| n.trim_start_matches('/').to_string())
                .unwrap_or_else(|| c.id[..12.min(c.id.len())].into())
        });
        let runtime = runtime_kind(kind, compose.is_some());
        let mut workload = Workload {
            id: wid.clone(),
            display_name: name,
            state: if c.state == "running" {
                WorkloadState::Running
            } else {
                WorkloadState::Unknown
            },
            runtime,
            pids: vec![],
            listeners: vec![],
            project: None,
            manager: Some(manager.clone()),
            source: None,
            actions: vec![],
            health: Some(c.status.clone()),
            metrics: Some(c.image.clone()),
            restart_policy: None,
            lazyadmin_run_id: None,
            provenance: vec![prov(
                "container API reports container",
                format!("id={} labels={:?}", c.id, c.labels),
                Confidence::High,
            )],
        };
        for p in &c.ports {
            if let Some(public) = p.public_port {
                let proto = if p.typ.as_deref() == Some("udp") {
                    Protocol::Udp
                } else {
                    Protocol::Tcp
                };
                let ip = p.ip.clone().unwrap_or_else(|| "0.0.0.0".into());
                let lid = ListenerId::new(format!(
                    "container:{}:{}:{}:{}",
                    &c.id[..12.min(c.id.len())],
                    ip,
                    public,
                    if proto == Protocol::Udp { "udp" } else { "tcp" }
                ));
                workload.listeners.push(lid.clone());
                let exposure = exposure_for(&ip);
                let listener = Listener {
                    id: lid.clone(),
                    protocol: proto,
                    family: if ip.contains(':') {
                        AddressFamily::Ipv6
                    } else {
                        AddressFamily::Ipv4
                    },
                    bind_addr: Some(ip.clone()),
                    port: Some(public),
                    path: None,
                    state: ListenerState::Listen,
                    netns: "container-api".into(),
                    socket_inode: None,
                    exposure: exposure.clone(),
                    owners: vec![EntityRef::Workload(wid.clone())],
                    confidence: Confidence::High,
                    provenance: vec![prov(
                        "container API reports binding",
                        format!("{}:{} -> container:{}", ip, public, p.private_port),
                        Confidence::High,
                    )],
                    first_seen: Utc::now(),
                    last_seen: Utc::now(),
                    dual_stack_state: if ip.contains(':') {
                        DualStackState::Unknown
                    } else {
                        DualStackState::NotApplicable
                    },
                };
                if exposure != Exposure::Loopback {
                    out.warnings.push(warn(
                        "PUBLIC",
                        format!("published beyond localhost: {ip}:{public}"),
                        Some(EntityRef::Listener(lid.clone())),
                    ));
                }
                out.edges.push(edge(
                    EdgeKind::WorkloadOwnsListener,
                    EntityRef::Workload(wid.clone()),
                    EntityRef::Listener(lid.clone()),
                    "published port",
                ));
                out.listeners.push(listener);
            }
        }
        out.edges.push(edge(
            EdgeKind::ManagerOwnsWorkload,
            EntityRef::Manager(manager.clone()),
            EntityRef::Workload(wid.clone()),
            "container manager",
        ));
        out.workloads.push(workload);
        if let Some(wd) = c.labels.get("com.docker.compose.project.working_dir") {
            out.projects
                .push(project_from_root(PathBuf::from(wd), "compose working_dir"));
            out.edges.push(edge(
                EdgeKind::WorkloadInProject,
                EntityRef::Workload(wid),
                EntityRef::Project(ProjectId::new(wd)),
                "compose working_dir",
            ));
        }
    }
    out
}
fn exposure_for(ip: &str) -> Exposure {
    match ip {
        "127.0.0.1" | "::1" | "localhost" => Exposure::Loopback,
        "" => Exposure::Unknown,
        _ => Exposure::LanOrPublic,
    }
}
fn project_from_root(root: PathBuf, e: &str) -> Project {
    let name = root
        .file_name()
        .and_then(|s| s.to_str())
        .unwrap_or("project")
        .to_string();
    Project {
        id: ProjectId::new(root.to_string_lossy()),
        root,
        name,
        markers: vec![],
        git_remote: None,
        package_manager: None,
        dev_commands: vec![],
        provenance: vec![prov("project hint", e, Confidence::High)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn endpoint_config() {
        assert!(
            default_endpoints()
                .iter()
                .any(|e| e.source.contains("docker.sock"))
        );
    }
    #[test]
    fn discovery_published_ports() {
        let json = include_str!("../../../testdata/container/docker-list.json");
        let out = discover_from_docker_list_json(
            json,
            &ContainerRuntimeKind::Docker,
            ManagerId::new("m"),
        )
        .unwrap();
        assert_eq!(out.workloads.len(), 2);
        assert!(out.listeners.iter().any(|l| l.port == Some(5432)));
        assert!(out.warnings.iter().any(|w| w.code == "PUBLIC"));
    }
    #[test]
    fn compose_stable() {
        let json = include_str!("../../../testdata/container/compose-list.json");
        let out = discover_from_docker_list_json(
            json,
            &ContainerRuntimeKind::Docker,
            ManagerId::new("m"),
        )
        .unwrap();
        assert_eq!(out.workloads[0].id.0, "compose:acme/web");
    }

    #[tokio::test]
    async fn events_watch_stream_is_deferred() {
        use lazyadmin_core::graph::DiscoveryAdapter;
        let adapter = ContainerAdapter::new();
        assert!(adapter.watch().await.is_none());
        assert!(!adapter.capabilities().watching);
    }
}
