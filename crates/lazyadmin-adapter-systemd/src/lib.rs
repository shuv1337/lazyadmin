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
use std::{fs, time::Duration};

#[derive(Clone, Debug, Default)]
pub struct SystemdAdapter;
fn prov(claim: &str, evidence: impl Into<String>, confidence: Confidence) -> Provenance {
    Provenance {
        adapter: "systemd".into(),
        claim: claim.into(),
        evidence: evidence.into(),
        confidence,
        timestamp: Utc::now(),
    }
}
pub fn unit_from_cgroup(cgroup: &str) -> Option<(RuntimeKind, String)> {
    for line in cgroup.lines() {
        for part in line.split('/') {
            if part.ends_with(".service") || part.ends_with(".scope") || part.ends_with(".socket") {
                let u = part.replace("\\x2d", "-");
                let kind = if line.contains("user.slice") {
                    RuntimeKind::SystemdUser
                } else {
                    RuntimeKind::SystemdSystem
                };
                return Some((kind, u));
            }
        }
    }
    None
}
pub fn parse_restart_policy(raw: &str) -> RestartPolicy {
    RestartPolicy {
        source: RestartPolicySource::SystemdRestart,
        policy: raw.trim().into(),
        raw: raw.into(),
    }
}
#[async_trait]
impl DiscoveryAdapter for SystemdAdapter {
    fn name(&self) -> &'static str {
        "systemd"
    }
    fn capabilities(&self) -> AdapterCapabilities {
        AdapterCapabilities {
            polling: true,
            watching: false,
        }
    }
    #[tracing::instrument(name = "adapter.systemd.health", skip_all)]
    async fn health(&self) -> AdapterHealth {
        let user = tokio::time::timeout(Duration::from_millis(300), zbus::Connection::session())
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false);
        let system = tokio::time::timeout(Duration::from_millis(300), zbus::Connection::system())
            .await
            .map(|r| r.is_ok())
            .unwrap_or(false);
        AdapterHealth {
            adapter: "systemd".into(),
            available: user || system,
            message: Some(format!(
                "user_bus_reachable={user} system_bus_reachable={system} journal_availability=unknown method_availability=runtime_detected polkit_may_be_required=true"
            )),
        }
    }
    #[tracing::instrument(name = "adapter.systemd.discover", skip_all)]
    async fn discover(&self, _: DiscoveryContext) -> anyhow::Result<DiscoveryOutput> {
        let mut out = DiscoveryOutput::default();
        out.managers.push(Manager {
            id: ManagerId::new("systemd-system"),
            kind: RuntimeKind::SystemdSystem,
            name: "systemd system".into(),
            scope: ManagerScope::System,
            socket: None,
            available: true,
            permission: PermissionState::Unknown,
            version: None,
            provenance: vec![prov("manager", "system bus", Confidence::Medium)],
        });
        out.managers.push(Manager {
            id: ManagerId::new("systemd-user"),
            kind: RuntimeKind::SystemdUser,
            name: "systemd user".into(),
            scope: ManagerScope::User,
            socket: None,
            available: true,
            permission: PermissionState::Unknown,
            version: None,
            provenance: vec![prov("manager", "user bus", Confidence::Medium)],
        });
        if let Ok(rd) = fs::read_dir("/proc") {
            for e in rd.flatten() {
                let Some(pid) = e.file_name().to_str().and_then(|s| s.parse::<i32>().ok()) else {
                    continue;
                };
                if let Ok(cg) = fs::read_to_string(e.path().join("cgroup")) {
                    if let Some((kind, unit)) = unit_from_cgroup(&cg) {
                        let wid = WorkloadId::new(format!("systemd:{unit}"));
                        out.workloads.push(Workload {
                            id: wid.clone(),
                            display_name: unit.clone(),
                            runtime: kind.clone(),
                            state: WorkloadState::Running,
                            pids: vec![],
                            listeners: vec![],
                            project: None,
                            manager: Some(match kind {
                                RuntimeKind::SystemdUser => ManagerId::new("systemd-user"),
                                _ => ManagerId::new("systemd-system"),
                            }),
                            source: None,
                            actions: vec![],
                            health: None,
                            metrics: None,
                            restart_policy: Some(parse_restart_policy("unknown")),
                            lazyadmin_run_id: None,
                            provenance: vec![prov(
                                "cgroup unit",
                                format!("pid {pid}"),
                                Confidence::Medium,
                            )],
                        });
                    }
                }
            }
        }
        Ok(out)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cgroup_unit_lookup() {
        assert_eq!(
            unit_from_cgroup("0::/system.slice/ssh.service").unwrap().1,
            "ssh.service"
        );
    }
    #[test]
    fn restart_policy() {
        assert_eq!(parse_restart_policy("always").policy, "always");
    }
}
