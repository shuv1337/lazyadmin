use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::{fmt, path::PathBuf};
use uuid::Uuid;

pub const SNAPSHOT_SCHEMA_VERSION: &str = "lazyadmin.snapshot.v1";
pub const DIFF_SCHEMA_VERSION: &str = "lazyadmin.diff.v1";
pub const DOCTOR_SCHEMA_VERSION: &str = "lazyadmin.doctor.v1";
pub const DISCOVERY_EVENT_SCHEMA_VERSION: &str = "lazyadmin.discovery_event.v1";

macro_rules! id_type {
    ($name:ident) => {
        #[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(pub String);
        impl $name {
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }
            pub fn uuid_v7() -> Self {
                Self(Uuid::now_v7().to_string())
            }
        }
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_tuple(stringify!($name)).field(&self.0).finish()
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}
id_type!(ListenerId);
id_type!(ProcessId);
id_type!(WorkloadId);
id_type!(ManagerId);
id_type!(ProjectId);
id_type!(RunId);
id_type!(ActionId);

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind", content = "id")]
pub enum EntityRef {
    Listener(ListenerId),
    Process(ProcessRef),
    Workload(WorkloadId),
    Manager(ManagerId),
    Project(ProjectId),
    Run(RunId),
    Action(ActionId),
}
pub type ProcessRef = ProcessKey;
pub type WorkloadRef = WorkloadId;
pub type ManagerRef = ManagerId;
pub type ProjectRef = ProjectId;
pub type ListenerRef = ListenerId;
pub type NamespaceId = String;

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ProcessKey {
    pub pid: i32,
    pub boot_id: String,
    pub start_time_ticks: u64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Confidence {
    Low,
    Medium,
    High,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RuntimeKind {
    Direct,
    LazyadminTracked,
    SystemdSystem,
    SystemdUser,
    SystemdSocket,
    Docker,
    DockerCompose,
    Portless,
    Podman,
    PodmanCompose,
    PodmanPod,
    Launchd,
    Supervisor,
    KubectlPortForward,
    SshTunnel,
    Socat,
    Cloudflared,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Exposure {
    Loopback,
    LanOrPublic,
    Public,
    ContainerOnly,
    UnixLocal,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Protocol {
    Tcp,
    Udp,
    Unix,
    Any,
}
#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum AddressFamily {
    Ipv4,
    Ipv6,
    Unix,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListenerState {
    Listen,
    Bound,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DualStackState {
    NotApplicable,
    ConfirmedDualStack,
    ConfirmedV6Only,
    Possible,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkloadState {
    Running,
    Stopped,
    Exited,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagerScope {
    System,
    User,
    Container,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PermissionState {
    Ok,
    Partial,
    Denied,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DangerLevel {
    Safe,
    Warn,
    Destructive,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RestartPolicySource {
    SystemdRestart,
    DockerRestart,
    PodmanRestart,
    None,
    Unknown,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningSeverity {
    Info,
    Warning,
    Error,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EdgeKind {
    ProcessOwnsListener,
    WorkloadContainsProcess,
    WorkloadOwnsListener,
    ManagerOwnsWorkload,
    WorkloadInProject,
    WorkloadActivatedBy,
    TrackedRunSpawned,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Provenance {
    pub adapter: String,
    pub claim: String,
    pub evidence: String,
    pub confidence: Confidence,
    pub timestamp: DateTime<Utc>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Listener {
    pub id: ListenerId,
    pub protocol: Protocol,
    pub family: AddressFamily,
    pub bind_addr: Option<String>,
    pub port: Option<u16>,
    pub path: Option<PathBuf>,
    pub state: ListenerState,
    pub netns: NamespaceId,
    pub socket_inode: Option<u64>,
    pub exposure: Exposure,
    pub owners: Vec<EntityRef>,
    pub confidence: Confidence,
    pub provenance: Vec<Provenance>,
    pub first_seen: DateTime<Utc>,
    pub last_seen: DateTime<Utc>,
    #[serde(default = "default_dual_stack_state")]
    pub dual_stack_state: DualStackState,
}
fn default_dual_stack_state() -> DualStackState {
    DualStackState::Unknown
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct RedactedEnvironmentSummary {
    pub keys: Vec<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Process {
    pub key: ProcessKey,
    pub pid: i32,
    pub start_time_ticks: u64,
    pub boot_id: String,
    pub user: Option<String>,
    pub exe: Option<PathBuf>,
    pub cmdline: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub ppid: Option<i32>,
    pub pgid: Option<i32>,
    pub sid: Option<i32>,
    pub cgroup: Option<String>,
    pub netns: Option<NamespaceId>,
    pub container_id: Option<String>,
    pub systemd_unit: Option<String>,
    pub lazyadmin_run_id: Option<RunId>,
    pub environment: RedactedEnvironmentSummary,
    pub provenance: Vec<Provenance>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RestartPolicy {
    pub source: RestartPolicySource,
    pub policy: String,
    pub raw: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub id: ActionId,
    pub label: String,
    pub danger: DangerLevel,
    pub target: EntityRef,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Workload {
    pub id: WorkloadId,
    pub display_name: String,
    pub runtime: RuntimeKind,
    pub state: WorkloadState,
    pub pids: Vec<ProcessRef>,
    pub listeners: Vec<ListenerRef>,
    pub project: Option<ProjectRef>,
    pub manager: Option<ManagerRef>,
    pub source: Option<EntityRef>,
    pub actions: Vec<Action>,
    pub health: Option<String>,
    pub metrics: Option<String>,
    pub restart_policy: Option<RestartPolicy>,
    pub lazyadmin_run_id: Option<RunId>,
    pub provenance: Vec<Provenance>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Manager {
    pub id: ManagerId,
    pub kind: RuntimeKind,
    pub name: String,
    pub scope: ManagerScope,
    pub socket: Option<PathBuf>,
    pub available: bool,
    pub permission: PermissionState,
    pub version: Option<String>,
    pub provenance: Vec<Provenance>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectMarker {
    pub kind: String,
    pub path: PathBuf,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DevCommandHint {
    pub name: String,
    pub command: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Project {
    pub id: ProjectId,
    pub root: PathBuf,
    pub name: String,
    pub markers: Vec<ProjectMarker>,
    pub git_remote: Option<String>,
    pub package_manager: Option<String>,
    pub dev_commands: Vec<DevCommandHint>,
    pub provenance: Vec<Provenance>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackedRun {
    pub id: RunId,
    pub tag: Option<String>,
    pub command: Vec<String>,
    pub cwd: Option<PathBuf>,
    pub state: WorkloadState,
    pub started_at: Option<DateTime<Utc>>,
    pub provenance: Vec<Provenance>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Edge {
    pub kind: EdgeKind,
    pub from: EntityRef,
    pub to: EntityRef,
    pub provenance: Vec<Provenance>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Warning {
    pub severity: WarningSeverity,
    pub code: String,
    pub message: String,
    pub entity: Option<EntityRef>,
    pub provenance: Vec<Provenance>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Host {
    pub boot_id: Option<String>,
    pub hostname: Option<String>,
    pub kernel: Option<String>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotMetadata {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events_dropped: Option<u64>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiscoveryEventKind {
    Added,
    Removed,
    Changed,
    Heartbeat,
    Degraded,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldChange {
    pub field: String,
    pub old: String,
    pub new: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiscoveryEvent {
    pub schema_version: String,
    pub kind: DiscoveryEventKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub entity: Option<EntityRef>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub changes: Option<Vec<FieldChange>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub adapter: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reason: Option<String>,
    pub timestamp: DateTime<Utc>,
}
impl DiscoveryEvent {
    pub fn added(entity: EntityRef) -> Self {
        Self::new(DiscoveryEventKind::Added, Some(entity), None, None, None)
    }
    pub fn removed(entity: EntityRef) -> Self {
        Self::new(DiscoveryEventKind::Removed, Some(entity), None, None, None)
    }
    pub fn changed(entity: EntityRef, changes: Vec<FieldChange>) -> Self {
        Self::new(
            DiscoveryEventKind::Changed,
            Some(entity),
            Some(changes),
            None,
            None,
        )
    }
    pub fn heartbeat(adapter: impl Into<String>) -> Self {
        Self::new(
            DiscoveryEventKind::Heartbeat,
            None,
            None,
            Some(adapter.into()),
            None,
        )
    }
    pub fn degraded(adapter: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::new(
            DiscoveryEventKind::Degraded,
            None,
            None,
            Some(adapter.into()),
            Some(reason.into()),
        )
    }
    fn new(
        kind: DiscoveryEventKind,
        entity: Option<EntityRef>,
        changes: Option<Vec<FieldChange>>,
        adapter: Option<String>,
        reason: Option<String>,
    ) -> Self {
        Self {
            schema_version: DISCOVERY_EVENT_SCHEMA_VERSION.into(),
            kind,
            entity,
            changes,
            adapter,
            reason,
            timestamp: Utc::now(),
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Snapshot {
    pub schema_version: String,
    pub generated_at: DateTime<Utc>,
    pub host: Host,
    pub managers: Vec<Manager>,
    pub processes: Vec<Process>,
    pub listeners: Vec<Listener>,
    pub workloads: Vec<Workload>,
    pub projects: Vec<Project>,
    pub tracked_runs: Vec<TrackedRun>,
    pub edges: Vec<Edge>,
    pub warnings: Vec<Warning>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub metadata: Option<SnapshotMetadata>,
}
impl Snapshot {
    pub fn empty() -> Self {
        Self {
            schema_version: SNAPSHOT_SCHEMA_VERSION.into(),
            generated_at: Utc::now(),
            host: Host {
                boot_id: None,
                hostname: None,
                kernel: None,
            },
            managers: vec![],
            processes: vec![],
            listeners: vec![],
            workloads: vec![],
            projects: vec![],
            tracked_runs: vec![],
            edges: vec![],
            warnings: vec![],
            metadata: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn discovery_event_schema_version_serializes() {
        let event = DiscoveryEvent::heartbeat("procfs");
        let json = serde_json::to_value(&event).unwrap();
        assert_eq!(json["schema_version"], DISCOVERY_EVENT_SCHEMA_VERSION);
        assert_eq!(json["kind"], "heartbeat");
    }

    #[test]
    fn discovery_event_added_helper_carries_entity_and_no_changes() {
        let entity = EntityRef::Listener(ListenerId::new("l"));
        let event = DiscoveryEvent::added(entity.clone());
        assert!(matches!(event.kind, DiscoveryEventKind::Added));
        assert_eq!(event.entity, Some(entity));
        assert!(event.changes.is_none());
        assert!(event.adapter.is_none());
        assert!(event.reason.is_none());
    }

    #[test]
    fn discovery_event_changed_helper_carries_changes() {
        let entity = EntityRef::Workload(WorkloadId::new("w"));
        let event = DiscoveryEvent::changed(
            entity.clone(),
            vec![FieldChange {
                field: "state".into(),
                old: "running".into(),
                new: "stopped".into(),
            }],
        );
        assert!(matches!(event.kind, DiscoveryEventKind::Changed));
        let changes = event.changes.as_ref().unwrap();
        assert_eq!(changes.len(), 1);
        assert_eq!(changes[0].field, "state");
    }

    #[test]
    fn discovery_event_removed_helper_no_changes_or_adapter() {
        let entity = EntityRef::Listener(ListenerId::new("l"));
        let event = DiscoveryEvent::removed(entity);
        assert!(matches!(event.kind, DiscoveryEventKind::Removed));
        assert!(event.changes.is_none());
    }

    #[test]
    fn discovery_event_degraded_carries_adapter_and_reason() {
        let event = DiscoveryEvent::degraded("procfs", "socket open failed");
        assert!(matches!(event.kind, DiscoveryEventKind::Degraded));
        assert_eq!(event.adapter.as_deref(), Some("procfs"));
        assert_eq!(event.reason.as_deref(), Some("socket open failed"));
    }

    #[test]
    fn snapshot_empty_has_canonical_schema_version_and_no_entities() {
        let s = Snapshot::empty();
        assert_eq!(s.schema_version, SNAPSHOT_SCHEMA_VERSION);
        assert!(s.listeners.is_empty());
        assert!(s.workloads.is_empty());
        assert!(s.processes.is_empty());
        assert!(s.managers.is_empty());
        assert!(s.projects.is_empty());
        assert!(s.tracked_runs.is_empty());
        assert!(s.edges.is_empty());
        assert!(s.warnings.is_empty());
        assert!(s.metadata.is_none());
    }

    #[test]
    fn id_types_display_uses_inner_string() {
        let id = ListenerId::new("tcp:0:0");
        assert_eq!(format!("{id}"), "tcp:0:0");
        assert_eq!(format!("{:?}", id), "ListenerId(\"tcp:0:0\")");
    }

    #[test]
    fn id_types_uuid_v7_is_unique_per_call() {
        let a = WorkloadId::uuid_v7();
        let b = WorkloadId::uuid_v7();
        assert_ne!(a, b);
    }

    #[test]
    fn entity_ref_round_trips_as_tagged_json() {
        let r = EntityRef::Listener(ListenerId::new("x"));
        let json = serde_json::to_string(&r).unwrap();
        // serde tag = "kind", content = "id"
        assert!(json.contains("\"kind\":\"listener\""));
        assert!(json.contains("\"id\":\"x\""));
        let back: EntityRef = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn confidence_orders_high_above_low() {
        assert!(Confidence::High > Confidence::Medium);
        assert!(Confidence::Medium > Confidence::Low);
    }

    #[test]
    fn runtime_kind_round_trips_in_snake_case() {
        for kind in [
            RuntimeKind::Direct,
            RuntimeKind::Docker,
            RuntimeKind::DockerCompose,
            RuntimeKind::Portless,
            RuntimeKind::SystemdUser,
            RuntimeKind::Unknown,
        ] {
            let json = serde_json::to_string(&kind).unwrap();
            let back: RuntimeKind = serde_json::from_str(&json).unwrap();
            assert_eq!(back, kind);
        }
    }

    #[test]
    fn exposure_round_trips_in_snake_case() {
        for ex in [
            Exposure::Public,
            Exposure::LanOrPublic,
            Exposure::Loopback,
            Exposure::ContainerOnly,
            Exposure::UnixLocal,
            Exposure::Unknown,
        ] {
            let json = serde_json::to_string(&ex).unwrap();
            let back: Exposure = serde_json::from_str(&json).unwrap();
            assert_eq!(back, ex);
        }
    }

    #[test]
    fn warning_severity_round_trips() {
        for s in [
            WarningSeverity::Info,
            WarningSeverity::Warning,
            WarningSeverity::Error,
        ] {
            let json = serde_json::to_string(&s).unwrap();
            let back: WarningSeverity = serde_json::from_str(&json).unwrap();
            assert_eq!(back, s);
        }
    }

    #[test]
    fn process_key_hash_eq_uses_all_fields() {
        use std::collections::HashSet;
        let a = ProcessKey {
            pid: 1,
            boot_id: "b".into(),
            start_time_ticks: 7,
        };
        let b = ProcessKey {
            pid: 1,
            boot_id: "b".into(),
            start_time_ticks: 7,
        };
        let c = ProcessKey {
            pid: 1,
            boot_id: "b".into(),
            start_time_ticks: 8,
        };
        assert_eq!(a, b);
        let mut set = HashSet::new();
        set.insert(a.clone());
        assert!(set.contains(&b));
        assert!(!set.contains(&c));
    }

    #[test]
    fn listener_defaults_dual_stack_unknown_for_old_json() {
        let json = serde_json::json!({
            "id": "listener:test",
            "protocol": "tcp",
            "family": "ipv4",
            "bind_addr": "127.0.0.1",
            "port": 3000,
            "path": null,
            "state": "listen",
            "netns": "host",
            "socket_inode": null,
            "exposure": "loopback",
            "owners": [],
            "confidence": "high",
            "provenance": [],
            "first_seen": "2026-04-27T12:00:00Z",
            "last_seen": "2026-04-27T12:00:00Z"
        });
        let listener: Listener = serde_json::from_value(json).unwrap();
        assert_eq!(listener.dual_stack_state, DualStackState::Unknown);
    }
}
