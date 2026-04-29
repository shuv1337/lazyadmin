#![forbid(unsafe_code)]
#![deny(missing_debug_implementations)]

use async_trait::async_trait;
use chrono::Utc;
use lazyadmin_core::{
    graph::{
        AdapterCapabilities, AdapterHealth, DiscoveryAdapter, DiscoveryContext, DiscoveryOutput,
    },
    model::*,
};
use serde::Deserialize;
use std::{
    env, io,
    path::{Path, PathBuf},
    process::Command,
};

const LEGACY_STATE_DIR: &str = "/tmp/portless";

#[derive(Clone, Debug)]
pub struct PortlessAdapter {
    state_dirs: Vec<StateDir>,
    portless_bin: Option<PathBuf>,
    version: Option<String>,
}

#[derive(Clone, Debug)]
struct StateDir {
    path: PathBuf,
    legacy: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PortlessRoute {
    pub hostname: String,
    pub port: u16,
    pub pid: i32,
}

#[derive(Clone, Debug, Deserialize)]
struct RouteFileEntry {
    hostname: Option<String>,
    port: Option<u64>,
    pid: Option<i64>,
}

impl PortlessAdapter {
    #[must_use]
    pub fn new() -> Self {
        let bin = find_portless_binary();
        Self {
            state_dirs: resolve_state_dirs_from_env(),
            version: bin.as_ref().and_then(|path| portless_version(path)),
            portless_bin: bin,
        }
    }

    #[must_use]
    pub fn with_state_dirs_for_test(
        state_dirs: Vec<PathBuf>,
        portless_bin: Option<PathBuf>,
    ) -> Self {
        Self {
            state_dirs: state_dirs
                .into_iter()
                .map(|path| StateDir {
                    legacy: path == Path::new(LEGACY_STATE_DIR),
                    path,
                })
                .collect(),
            version: portless_bin
                .as_ref()
                .and_then(|path| portless_version(path)),
            portless_bin,
        }
    }
}

impl Default for PortlessAdapter {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl DiscoveryAdapter for PortlessAdapter {
    fn name(&self) -> &'static str {
        "portless"
    }

    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            polling: true,
            watching: false,
        }
    }

    async fn health(&self) -> AdapterHealth {
        AdapterHealth {
            adapter: "portless".into(),
            available: self.state_dirs.iter().any(|dir| dir.path.exists()),
            message: Some(format!(
                "state_dirs={} binary={}",
                self.state_dirs
                    .iter()
                    .map(|dir| dir.path.display().to_string())
                    .collect::<Vec<_>>()
                    .join(","),
                self.portless_bin
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_else(|| "missing".into())
            )),
        }
    }

    #[tracing::instrument(name = "adapter.portless.discover", skip_all)]
    async fn discover(&self, _: DiscoveryContext) -> anyhow::Result<DiscoveryOutput> {
        let mut out = DiscoveryOutput::default();
        let existing_dirs: Vec<_> = self
            .state_dirs
            .iter()
            .filter(|dir| dir.path.exists())
            .cloned()
            .collect();
        if existing_dirs.is_empty() {
            return Ok(out);
        }

        for state_dir in existing_dirs {
            let manager_id = ManagerId::new(format!("portless:{}", state_dir.path.display()));
            let routes_path = state_dir.path.join("routes.json");
            let (routes, permission) = match tokio::fs::read(&routes_path).await {
                Ok(bytes) => (
                    parse_routes(&bytes, &state_dir.path, &mut out.warnings),
                    PermissionState::Ok,
                ),
                Err(err) if err.kind() == io::ErrorKind::NotFound => {
                    (Vec::new(), PermissionState::Unknown)
                }
                Err(err) if err.kind() == io::ErrorKind::PermissionDenied => {
                    out.warnings.push(warn(
                        "portless.routes_unreadable",
                        format!("{} is not readable: {err}", routes_path.display()),
                        Some(EntityRef::Manager(manager_id.clone())),
                    ));
                    (Vec::new(), PermissionState::Denied)
                }
                Err(err) => {
                    out.warnings.push(warn(
                        "portless.routes_unreadable",
                        format!("{} could not be read: {err}", routes_path.display()),
                        Some(EntityRef::Manager(manager_id.clone())),
                    ));
                    (Vec::new(), PermissionState::Unknown)
                }
            };

            out.managers.push(Manager {
                id: manager_id.clone(),
                kind: RuntimeKind::Portless,
                name: state_dir
                    .path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .map(|name| format!("portless {name}"))
                    .unwrap_or_else(|| "portless".into()),
                scope: ManagerScope::User,
                socket: None,
                available: self.portless_bin.is_some(),
                permission,
                version: self.version.clone(),
                provenance: vec![
                    prov(
                        "state dir",
                        format!("state dir = {}", state_dir.path.display()),
                        Confidence::High,
                    ),
                    prov(
                        "binary",
                        format!(
                            "binary = {}",
                            self.portless_bin
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|| "missing".into())
                        ),
                        Confidence::Medium,
                    ),
                    prov(
                        "legacy",
                        format!("legacy = {}", state_dir.legacy),
                        Confidence::High,
                    ),
                ],
            });

            for route in routes {
                if route.pid != 0 && !proc_pid_exists(route.pid) {
                    out.warnings.push(warn(
                        "portless.orphan_route",
                        format!(
                            "route '{}' :{} has dead CLI pid {}; run `portless prune` to clean up",
                            route.hostname, route.port, route.pid
                        ),
                        Some(EntityRef::Manager(manager_id.clone())),
                    ));
                    continue;
                }

                let workload_id = WorkloadId::new(format!(
                    "portless:{}:{}:{}",
                    state_dir.path.display(),
                    route.hostname,
                    route.port
                ));
                let evidence = if route.pid == 0 {
                    format!(
                        "static alias state_dir={} hostname={} port={}",
                        state_dir.path.display(),
                        route.hostname,
                        route.port
                    )
                } else {
                    format!(
                        "routes.json pid={} state_dir={} hostname={} port={}",
                        route.pid,
                        state_dir.path.display(),
                        route.hostname,
                        route.port
                    )
                };
                out.workloads.push(Workload {
                    id: workload_id.clone(),
                    display_name: route.hostname.clone(),
                    runtime: RuntimeKind::Portless,
                    state: WorkloadState::Running,
                    pids: vec![],
                    listeners: vec![],
                    project: None,
                    manager: Some(manager_id.clone()),
                    source: None,
                    actions: vec![],
                    health: None,
                    metrics: Some(format!("route_port={}", route.port)),
                    restart_policy: None,
                    lazyadmin_run_id: None,
                    provenance: vec![prov(
                        if route.pid == 0 {
                            "static alias"
                        } else {
                            "route cli pid"
                        },
                        evidence,
                        if route.pid == 0 {
                            Confidence::Medium
                        } else {
                            Confidence::High
                        },
                    )],
                });
                out.edges.push(Edge {
                    kind: EdgeKind::ManagerOwnsWorkload,
                    from: EntityRef::Manager(manager_id.clone()),
                    to: EntityRef::Workload(workload_id),
                    provenance: vec![prov(
                        "portless route table",
                        routes_path.display().to_string(),
                        Confidence::High,
                    )],
                });
            }
        }
        Ok(out)
    }
}

pub fn default_state_dirs() -> Vec<PathBuf> {
    resolve_state_dirs_from_env()
        .into_iter()
        .map(|dir| dir.path)
        .collect()
}

pub fn parse_routes(
    bytes: &[u8],
    state_dir: &Path,
    warnings: &mut Vec<Warning>,
) -> Vec<PortlessRoute> {
    let parsed = match serde_json::from_slice::<Vec<RouteFileEntry>>(bytes) {
        Ok(parsed) => parsed,
        Err(err) => {
            warnings.push(warn(
                "portless.routes_unparseable",
                format!(
                    "{} routes.json is not parseable: {err}",
                    state_dir.display()
                ),
                None,
            ));
            return Vec::new();
        }
    };
    let mut routes = Vec::new();
    let mut dropped = 0usize;
    for entry in parsed {
        let Some(hostname) = entry
            .hostname
            .filter(|hostname| !hostname.trim().is_empty())
        else {
            dropped += 1;
            continue;
        };
        let Some(port) = entry.port.and_then(|port| u16::try_from(port).ok()) else {
            dropped += 1;
            continue;
        };
        let Some(pid) = entry
            .pid
            .and_then(|pid| i32::try_from(pid).ok())
            .filter(|pid| *pid >= 0)
        else {
            dropped += 1;
            continue;
        };
        routes.push(PortlessRoute {
            hostname,
            port,
            pid,
        });
    }
    if dropped > 0 {
        warnings.push(warn(
            "portless.routes_invalid",
            format!(
                "{} routes.json dropped {dropped} invalid route entr{}",
                state_dir.display(),
                if dropped == 1 { "y" } else { "ies" }
            ),
            None,
        ));
    }
    routes
}

fn resolve_state_dirs_from_env() -> Vec<StateDir> {
    if let Some(path) = env::var_os("PORTLESS_STATE_DIR").map(PathBuf::from) {
        return vec![StateDir {
            path,
            legacy: false,
        }];
    }
    let mut dirs = Vec::new();
    if let Some(home) = env::var_os("HOME").map(PathBuf::from) {
        dirs.push(StateDir {
            path: home.join(".portless"),
            legacy: false,
        });
    }
    let legacy = PathBuf::from(LEGACY_STATE_DIR);
    if legacy.exists() {
        dirs.push(StateDir {
            path: legacy,
            legacy: true,
        });
    }
    dirs
}

fn find_portless_binary() -> Option<PathBuf> {
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|dir| dir.join("portless"))
        .find(|candidate| candidate.is_file())
}

fn portless_version(path: &Path) -> Option<String> {
    let output = Command::new(path).arg("--version").output().ok()?;
    if !output.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&output.stdout).trim().to_string();
    (!text.is_empty()).then_some(text)
}

fn proc_pid_exists(pid: i32) -> bool {
    Path::new("/proc").join(pid.to_string()).exists()
}

fn prov(claim: &str, evidence: impl Into<String>, confidence: Confidence) -> Provenance {
    Provenance {
        adapter: "portless".into(),
        claim: claim.into(),
        evidence: evidence.into(),
        confidence,
        timestamp: Utc::now(),
    }
}

fn warn(code: &str, message: impl Into<String>, entity: Option<EntityRef>) -> Warning {
    Warning {
        severity: WarningSeverity::Warning,
        code: code.into(),
        message: message.into(),
        entity,
        provenance: vec![prov("warning", code, Confidence::Medium)],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use lazyadmin_core::graph::DiscoveryAdapter;
    use std::fs;

    fn routes_json(pid: i32) -> String {
        format!(
            r#"[
                {{"hostname":"live","port":3737,"pid":{pid},"extra":"ignored"}},
                {{"hostname":"alias","port":4444,"pid":0}},
                {{"hostname":"orphan","port":5555,"pid":999999999}},
                {{"hostname":"bad","port":70000,"pid":1}}
            ]"#
        )
    }

    #[test]
    fn parse_routes_accepts_unknown_fields_and_drops_invalid() {
        let mut warnings = Vec::new();
        let routes = parse_routes(
            routes_json(std::process::id() as i32).as_bytes(),
            Path::new("/state"),
            &mut warnings,
        );
        assert_eq!(routes.len(), 3);
        assert!(
            routes
                .iter()
                .any(|route| route.hostname == "alias" && route.pid == 0)
        );
        assert!(
            warnings
                .iter()
                .any(|warning| warning.code == "portless.routes_invalid")
        );
    }

    #[test]
    fn parse_routes_warns_on_malformed_json() {
        let mut warnings = Vec::new();
        let routes = parse_routes(b"{not json", Path::new("/state"), &mut warnings);
        assert!(routes.is_empty());
        assert!(
            warnings
                .iter()
                .any(|warning| warning.code == "portless.routes_unparseable")
        );
    }

    #[tokio::test]
    async fn discover_emits_manager_live_alias_and_orphan_warning() {
        let temp = tempfile::tempdir().unwrap();
        fs::write(
            temp.path().join("routes.json"),
            routes_json(std::process::id() as i32),
        )
        .unwrap();
        let adapter =
            PortlessAdapter::with_state_dirs_for_test(vec![temp.path().to_path_buf()], None);
        let out = adapter.discover(DiscoveryContext::default()).await.unwrap();
        assert_eq!(out.managers.len(), 1);
        assert_eq!(out.workloads.len(), 2);
        assert!(
            out.workloads
                .iter()
                .any(|workload| workload.display_name == "live")
        );
        assert!(
            out.workloads
                .iter()
                .any(|workload| workload.display_name == "alias")
        );
        assert!(
            out.warnings
                .iter()
                .any(|warning| warning.code == "portless.orphan_route")
        );
        assert!(
            out.edges
                .iter()
                .any(|edge| edge.kind == EdgeKind::ManagerOwnsWorkload)
        );
    }

    #[tokio::test]
    async fn discover_empty_when_state_dir_absent() {
        let temp = tempfile::tempdir().unwrap();
        let missing = temp.path().join("missing");
        let adapter = PortlessAdapter::with_state_dirs_for_test(vec![missing], None);
        let out = adapter.discover(DiscoveryContext::default()).await.unwrap();
        assert!(out.managers.is_empty());
        assert!(out.workloads.is_empty());
        assert!(out.warnings.is_empty());
    }
}
