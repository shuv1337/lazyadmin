#![forbid(unsafe_code)]
#![deny(missing_debug_implementations)]
use async_trait::async_trait;
use chrono::Utc;
use futures::{StreamExt, channel::mpsc, stream::BoxStream};
use lazyadmin_core::{
    graph::{
        AdapterCapabilities, AdapterHealth, DiscoveryAdapter, DiscoveryContext, DiscoveryOutput,
    },
    model::*,
};
use std::{fs, time::Duration};
use zbus::{MatchRule, Message, MessageStream, MessageType};

#[derive(Clone, Debug)]
pub struct SystemdAdapter {
    events_enabled: bool,
}

impl Default for SystemdAdapter {
    fn default() -> Self {
        Self {
            events_enabled: true,
        }
    }
}

impl SystemdAdapter {
    pub fn new(events_enabled: bool) -> Self {
        Self { events_enabled }
    }
}
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
            watching: self.events_enabled,
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

    #[tracing::instrument(name = "adapter.watch.start", skip_all, fields(adapter = "systemd"))]
    async fn watch(&self) -> Option<BoxStream<'static, DiscoveryEvent>> {
        if !self.events_enabled {
            return None;
        }
        let (tx, rx) = mpsc::unbounded();
        let mut started = false;
        for (scope, connection) in [
            ("system", zbus::Connection::system().await),
            ("user", zbus::Connection::session().await),
        ] {
            match connection {
                Ok(connection) => {
                    started = true;
                    let tx = tx.clone();
                    tokio::spawn(async move {
                        watch_systemd_connection(scope, connection, tx).await;
                    });
                }
                Err(err) => {
                    let _ = tx.unbounded_send(DiscoveryEvent::degraded(
                        "systemd",
                        format!("{scope} bus unavailable for systemd watch: {err}"),
                    ));
                }
            }
        }
        drop(tx);
        started.then_some(Box::pin(rx) as BoxStream<'static, DiscoveryEvent>)
    }
}

async fn watch_systemd_connection(
    scope: &'static str,
    connection: zbus::Connection,
    tx: mpsc::UnboundedSender<DiscoveryEvent>,
) {
    let mut streams = futures::stream::SelectAll::new();
    for rule in systemd_match_rules() {
        match MessageStream::for_match_rule(rule, &connection, Some(128)).await {
            Ok(stream) => streams.push(stream),
            Err(err) => {
                let _ = tx.unbounded_send(DiscoveryEvent::degraded(
                    "systemd",
                    format!("{scope} bus match registration failed: {err}"),
                ));
            }
        }
    }
    let _ = tx.unbounded_send(DiscoveryEvent::heartbeat("systemd"));
    while let Some(next) = streams.next().await {
        match next {
            Ok(message) => {
                if let Some(event) = systemd_message_to_discovery(scope, &message) {
                    tracing::debug!(name = "adapter.watch.event", adapter = "systemd", kind = ?event.kind);
                    let _ = tx.unbounded_send(event);
                }
            }
            Err(err) => {
                let _ = tx.unbounded_send(DiscoveryEvent::degraded(
                    "systemd",
                    format!("{scope} bus stream failed: {err}"),
                ));
                break;
            }
        }
    }
}

fn systemd_match_rules() -> Vec<MatchRule<'static>> {
    [
        "type='signal',sender='org.freedesktop.systemd1',interface='org.freedesktop.DBus.Properties',member='PropertiesChanged'",
        "type='signal',sender='org.freedesktop.systemd1',interface='org.freedesktop.systemd1.Manager',member='JobNew'",
        "type='signal',sender='org.freedesktop.systemd1',interface='org.freedesktop.systemd1.Manager',member='JobRemoved'",
    ]
    .into_iter()
    .filter_map(|rule| MatchRule::try_from(rule).ok().map(MatchRule::into_owned))
    .collect()
}

pub fn systemd_message_to_discovery(scope: &str, message: &Message) -> Option<DiscoveryEvent> {
    let header = message.header();
    if header.message_type() != MessageType::Signal {
        return None;
    }
    let member = header.member()?.as_str();
    let path = header.path().map(|p| p.as_str()).unwrap_or_default();
    systemd_signal_to_discovery(scope, member, path)
}

pub fn systemd_signal_to_discovery(
    scope: &str,
    member: &str,
    path: &str,
) -> Option<DiscoveryEvent> {
    let unit = unit_from_systemd_object_path(path).unwrap_or_else(|| format!("{scope}:activity"));
    let entity = EntityRef::Workload(WorkloadId::new(format!("systemd:{unit}")));
    let mut event = match member {
        "JobRemoved" => DiscoveryEvent::changed(
            entity,
            vec![FieldChange {
                field: "systemd.job".into(),
                old: String::new(),
                new: "removed".into(),
            }],
        ),
        "JobNew" => DiscoveryEvent::changed(
            entity,
            vec![FieldChange {
                field: "systemd.job".into(),
                old: String::new(),
                new: "new".into(),
            }],
        ),
        "PropertiesChanged" => DiscoveryEvent::changed(
            entity,
            vec![FieldChange {
                field: "systemd.properties".into(),
                old: String::new(),
                new: "changed".into(),
            }],
        ),
        _ => return None,
    };
    event.adapter = Some("systemd".into());
    Some(event)
}

pub fn unit_from_systemd_object_path(path: &str) -> Option<String> {
    let encoded = path.strip_prefix("/org/freedesktop/systemd1/unit/")?;
    Some(decode_systemd_unit_name(encoded))
}

fn decode_systemd_unit_name(encoded: &str) -> String {
    let mut out = String::new();
    let mut chars = encoded.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '_' {
            let a = chars.next();
            let b = chars.next();
            match (a, b) {
                (Some(a), Some(b)) if a.is_ascii_hexdigit() && b.is_ascii_hexdigit() => {
                    let hex = format!("{a}{b}");
                    if let Ok(byte) = u8::from_str_radix(&hex, 16) {
                        out.push(byte as char);
                    }
                }
                (Some(a), Some(b)) => {
                    out.push('_');
                    out.push(a);
                    out.push(b);
                }
                (Some(a), None) => {
                    out.push('_');
                    out.push(a);
                }
                _ => out.push('_'),
            }
        } else {
            out.push(ch);
        }
    }
    out
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

    #[test]
    fn events_map_systemd_signals() {
        assert_eq!(
            unit_from_systemd_object_path("/org/freedesktop/systemd1/unit/ssh_2eservice"),
            Some("ssh.service".into())
        );
        let event = systemd_signal_to_discovery(
            "system",
            "PropertiesChanged",
            "/org/freedesktop/systemd1/unit/ssh_2eservice",
        )
        .unwrap();
        assert_eq!(event.adapter.as_deref(), Some("systemd"));
        assert!(matches!(event.kind, DiscoveryEventKind::Changed));
        assert_eq!(
            event.entity,
            Some(EntityRef::Workload(WorkloadId::new("systemd:ssh.service")))
        );
    }

    #[tokio::test]
    async fn events_watch_stream_can_be_disabled() {
        use lazyadmin_core::graph::DiscoveryAdapter;
        let adapter = SystemdAdapter::new(false);
        assert!(adapter.watch().await.is_none());
        assert!(!adapter.capabilities().watching);
    }

    #[test]
    fn unit_from_cgroup_recognises_scope_and_socket_units() {
        let (kind, unit) = unit_from_cgroup("0::/system.slice/dev-foo.scope").unwrap();
        assert!(matches!(kind, RuntimeKind::SystemdSystem));
        assert_eq!(unit, "dev-foo.scope");

        let (kind, unit) = unit_from_cgroup("0::/user.slice/dev.socket").unwrap();
        assert!(matches!(kind, RuntimeKind::SystemdUser));
        assert_eq!(unit, "dev.socket");
    }

    #[test]
    fn unit_from_cgroup_returns_none_when_no_unit_segment() {
        assert!(unit_from_cgroup("0::/init.scope-without-suffix/abc").is_none());
        assert!(unit_from_cgroup("").is_none());
    }

    #[test]
    fn unit_from_cgroup_decodes_x2d_escapes() {
        let (_, unit) = unit_from_cgroup("0::/system.slice/dev\\x2dapi.service").unwrap();
        assert_eq!(unit, "dev-api.service");
    }

    #[test]
    fn unit_from_systemd_object_path_decodes_hex_escapes() {
        assert_eq!(
            unit_from_systemd_object_path("/org/freedesktop/systemd1/unit/foo_2dbar_2eservice"),
            Some("foo-bar.service".into())
        );
    }

    #[test]
    fn unit_from_systemd_object_path_returns_none_when_prefix_missing() {
        assert!(unit_from_systemd_object_path("/unrelated/path").is_none());
    }

    #[test]
    fn systemd_signal_to_discovery_returns_none_for_unknown_signals() {
        assert!(
            systemd_signal_to_discovery(
                "system",
                "NotARealSignal",
                "/org/freedesktop/systemd1/unit/ssh_2eservice",
            )
            .is_none()
        );
    }

    #[test]
    fn parse_restart_policy_trims_and_records_raw() {
        let r = parse_restart_policy("  on-failure\n");
        assert_eq!(r.policy, "on-failure");
        assert_eq!(r.raw, "  on-failure\n");
        assert!(matches!(r.source, RestartPolicySource::SystemdRestart));
    }

    #[test]
    fn adapter_name_and_default_polling() {
        let a = SystemdAdapter::default();
        assert_eq!(a.name(), "systemd");
        assert!(a.capabilities().polling);
        assert!(a.capabilities().watching, "default events_enabled is true");
    }
}
