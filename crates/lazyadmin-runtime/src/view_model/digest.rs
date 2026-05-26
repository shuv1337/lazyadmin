use std::collections::{BTreeMap, HashMap, HashSet};

use chrono::{DateTime, Utc};
use lazyadmin_core::model::{
    EntityRef, Exposure, Listener, ListenerId, Snapshot, WarningSeverity, Workload,
};
use serde::{Deserialize, Serialize};

use super::{
    doctor_groups::{TriageSummary, build_doctor_groups},
    relations::SnapshotRelations,
};

pub const EMPTY_EXPOSED: &str = "Nothing exposed beyond loopback. ✓";
pub const EMPTY_CONFLICTS: &str = "Nothing contended.";
pub const EMPTY_PROJECTS: &str = "No active projects detected.";
pub const EMPTY_TRIAGE: &str = "Everything's clean.";

pub const DIGEST_EXPOSED_LIMIT: usize = 10;
pub const DIGEST_CONFLICTS_LIMIT: usize = 5;
pub const DIGEST_PROJECTS_LIMIT: usize = 10;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Digest {
    pub exposed: ExposedSection,
    pub conflicts: ConflictsSection,
    pub your_projects: ProjectsSection,
    pub triage: TriageSection,
}

impl Digest {
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        build_digest(snapshot)
    }
}

pub fn build_digest(snapshot: &Snapshot) -> Digest {
    let doctor_groups = build_doctor_groups(snapshot);
    Digest {
        exposed: build_exposed_section(snapshot),
        conflicts: build_conflicts_section(snapshot),
        your_projects: build_projects_section(snapshot),
        triage: TriageSection {
            summary: doctor_groups.triage_summary(),
            empty_copy: EMPTY_TRIAGE.to_string(),
        },
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExposedSection {
    pub rows: Vec<ExposedRow>,
    pub total_public: usize,
    pub total_lan: usize,
    pub unowned_count: usize,
    pub view_all_target: DigestViewTarget,
    pub empty_copy: String,
}

impl Default for ExposedSection {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            total_public: 0,
            total_lan: 0,
            unowned_count: 0,
            view_all_target: DigestViewTarget::ListenersPublic,
            empty_copy: EMPTY_EXPOSED.to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExposedRow {
    pub listener_id: String,
    pub port: Option<u16>,
    pub bind: String,
    pub exposure: Exposure,
    pub owner_label: String,
    pub owner_pid: Option<i32>,
    pub project: Option<String>,
    pub extra_ports: usize,
    pub risk_rank: u8,
    pub unowned: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictsSection {
    pub rows: Vec<ConflictRow>,
    pub total: usize,
    pub view_all_target: DigestViewTarget,
    pub empty_copy: String,
}

impl Default for ConflictsSection {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            total: 0,
            view_all_target: DigestViewTarget::ListenersConflicts,
            empty_copy: EMPTY_CONFLICTS.to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConflictRow {
    pub listener_id: String,
    pub port: Option<u16>,
    pub bind: String,
    pub owner_count: usize,
    pub severity: WarningSeverity,
    pub reason: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectsSection {
    pub rows: Vec<ProjectRow>,
    pub total: usize,
    pub view_all_target: DigestViewTarget,
    pub empty_copy: String,
}

impl Default for ProjectsSection {
    fn default() -> Self {
        Self {
            rows: Vec::new(),
            total: 0,
            view_all_target: DigestViewTarget::Projects,
            empty_copy: EMPTY_PROJECTS.to_string(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRow {
    pub project_id: String,
    pub name: String,
    pub root: String,
    pub workload_count: usize,
    pub listener_count: usize,
    pub last_seen: Option<DateTime<Utc>>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriageSection {
    pub summary: TriageSummary,
    pub empty_copy: String,
}

impl Default for TriageSection {
    fn default() -> Self {
        Self {
            summary: TriageSummary {
                actionable: 0,
                noise_groups: 0,
                noise_total: 0,
                last_check: Utc::now(),
            },
            empty_copy: EMPTY_TRIAGE.to_string(),
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DigestViewTarget {
    #[default]
    ListenersPublic,
    ListenersConflicts,
    Projects,
    Doctor,
}

fn build_exposed_section(snapshot: &Snapshot) -> ExposedSection {
    let relations = SnapshotRelations::new(snapshot);
    let mut public = 0;
    let mut lan = 0;
    let mut unowned_count = 0;
    let mut candidates = Vec::new();

    for listener in snapshot
        .listeners
        .iter()
        .filter(|listener| matches!(listener.exposure, Exposure::LanOrPublic | Exposure::Public))
    {
        match listener.exposure {
            Exposure::Public => public += 1,
            Exposure::LanOrPublic => lan += 1,
            _ => {}
        }
        if listener.owners.is_empty() {
            unowned_count += 1;
        }
        candidates.push(exposed_row(listener, &relations));
    }

    candidates.sort_by_key(|row| {
        (
            !row.unowned,
            !matches!(row.exposure, Exposure::Public),
            row.project.is_none(),
            row.port.unwrap_or(u16::MAX),
            row.bind.clone(),
            row.listener_id.clone(),
        )
    });

    let mut deduped: Vec<ExposedRow> = Vec::new();
    let mut owner_index: HashMap<(i32, Exposure), usize> = HashMap::new();
    for row in candidates {
        if let Some(pid) = row.owner_pid {
            let key = (pid, row.exposure.clone());
            if let Some(index) = owner_index.get(&key).copied() {
                deduped[index].extra_ports += 1;
                continue;
            }
            owner_index.insert(key, deduped.len());
        }
        deduped.push(row);
    }
    deduped.truncate(DIGEST_EXPOSED_LIMIT);

    ExposedSection {
        rows: deduped,
        total_public: public,
        total_lan: lan,
        unowned_count,
        view_all_target: DigestViewTarget::ListenersPublic,
        empty_copy: EMPTY_EXPOSED.to_string(),
    }
}

fn exposed_row(listener: &Listener, relations: &SnapshotRelations<'_>) -> ExposedRow {
    let owner_pid = relations.listener_owner_pid(listener);
    let project = relations.listener_project_label(listener);
    let unowned = listener.owners.is_empty();
    let public = matches!(listener.exposure, Exposure::Public);
    let project_known = project.is_some();
    ExposedRow {
        listener_id: listener.id.to_string(),
        port: listener.port,
        bind: relations.listener_bind_label(listener),
        exposure: listener.exposure.clone(),
        owner_label: relations.listener_owner_label(listener, "—"),
        owner_pid,
        project,
        extra_ports: 0,
        risk_rank: risk_rank(unowned, public, project_known),
        unowned,
    }
}

fn build_conflicts_section(snapshot: &Snapshot) -> ConflictsSection {
    let relations = SnapshotRelations::new(snapshot);
    let mut warning_by_listener: BTreeMap<ListenerId, WarningSeverity> = BTreeMap::new();
    for warning in &snapshot.warnings {
        if warning.code != "CONFLICT" {
            continue;
        }
        if let Some(EntityRef::Listener(id)) = &warning.entity {
            warning_by_listener
                .entry(id.clone())
                .and_modify(|severity| {
                    if severity_rank(&warning.severity) < severity_rank(severity) {
                        *severity = warning.severity.clone();
                    }
                })
                .or_insert_with(|| warning.severity.clone());
        }
    }

    let mut rows = Vec::new();
    for listener in &snapshot.listeners {
        let warning_severity = warning_by_listener.get(&listener.id);
        if warning_severity.is_none() && listener.owners.len() <= 1 {
            continue;
        }
        let severity = warning_severity
            .cloned()
            .unwrap_or(WarningSeverity::Warning);
        rows.push(ConflictRow {
            listener_id: listener.id.to_string(),
            port: listener.port,
            bind: relations.listener_bind_label(listener),
            owner_count: listener.owners.len(),
            severity,
            reason: if warning_severity.is_some() {
                "listed in CONFLICT warning".to_string()
            } else {
                "multiple owners".to_string()
            },
        });
    }
    rows.sort_by_key(|row| {
        (
            severity_rank(&row.severity),
            std::cmp::Reverse(row.owner_count),
            row.port.unwrap_or(u16::MAX),
            row.bind.clone(),
        )
    });
    let total = rows.len();
    rows.truncate(DIGEST_CONFLICTS_LIMIT);
    ConflictsSection {
        rows,
        total,
        view_all_target: DigestViewTarget::ListenersConflicts,
        empty_copy: EMPTY_CONFLICTS.to_string(),
    }
}

fn build_projects_section(snapshot: &Snapshot) -> ProjectsSection {
    let listener_by_id: HashMap<_, _> = snapshot
        .listeners
        .iter()
        .map(|listener| (listener.id.clone(), listener))
        .collect();
    let mut rows = Vec::new();
    for project in &snapshot.projects {
        let workloads: Vec<&Workload> = snapshot
            .workloads
            .iter()
            .filter(|workload| workload.project.as_ref() == Some(&project.id))
            .collect();
        let mut listener_ids = HashSet::new();
        let mut last_seen = None;
        for workload in &workloads {
            for listener_id in &workload.listeners {
                if let Some(listener) = listener_by_id.get(listener_id) {
                    listener_ids.insert(listener.id.clone());
                    last_seen = max_time(last_seen, Some(listener.last_seen));
                }
            }
        }
        if listener_ids.is_empty() {
            continue;
        }
        rows.push(ProjectRow {
            project_id: project.id.to_string(),
            name: project.name.clone(),
            root: project.root.display().to_string(),
            workload_count: workloads.len(),
            listener_count: listener_ids.len(),
            last_seen,
        });
    }
    rows.sort_by_key(|row| {
        (
            std::cmp::Reverse(row.listener_count),
            std::cmp::Reverse(row.last_seen),
            row.name.clone(),
            row.project_id.clone(),
        )
    });
    let total = rows.len();
    rows.truncate(DIGEST_PROJECTS_LIMIT);
    ProjectsSection {
        rows,
        total,
        view_all_target: DigestViewTarget::Projects,
        empty_copy: EMPTY_PROJECTS.to_string(),
    }
}

fn risk_rank(unowned: bool, public: bool, project_known: bool) -> u8 {
    match (unowned, public, project_known) {
        (true, true, _) => 0,
        (true, false, _) => 1,
        (false, true, true) => 2,
        (false, true, false) => 3,
        (false, false, true) => 4,
        (false, false, false) => 5,
    }
}

fn severity_rank(severity: &WarningSeverity) -> u8 {
    match severity {
        WarningSeverity::Error => 0,
        WarningSeverity::Warning => 1,
        WarningSeverity::Info => 2,
    }
}

fn max_time(
    current: Option<DateTime<Utc>>,
    candidate: Option<DateTime<Utc>>,
) -> Option<DateTime<Utc>> {
    match (current, candidate) {
        (Some(a), Some(b)) => Some(a.max(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;
    use lazyadmin_core::model::{
        AddressFamily, Confidence, ListenerState, ProcessKey, Project, ProjectId, Protocol,
        Workload, WorkloadId, WorkloadState,
    };

    #[test]
    fn digest_empty_snapshot_has_all_affirmative_empty_states() {
        let snapshot = Snapshot::empty();
        let digest = build_digest(&snapshot);
        assert!(digest.exposed.rows.is_empty());
        assert_eq!(digest.exposed.empty_copy, EMPTY_EXPOSED);
        assert!(digest.conflicts.rows.is_empty());
        assert_eq!(digest.conflicts.empty_copy, EMPTY_CONFLICTS);
        assert!(digest.your_projects.rows.is_empty());
        assert_eq!(digest.your_projects.empty_copy, EMPTY_PROJECTS);
        assert_eq!(digest.triage.summary.actionable, 0);
        assert_eq!(digest.triage.empty_copy, EMPTY_TRIAGE);
    }

    #[test]
    fn exposed_dedupes_owner_pid_and_counts_extra_ports() {
        let mut snapshot = Snapshot::empty();
        let key = process_key(17385);
        snapshot.listeners.push(listener(
            "tcp:0.0.0.0:3000",
            "0.0.0.0",
            3000,
            Exposure::Public,
            vec![EntityRef::Process(key.clone())],
        ));
        snapshot.listeners.push(listener(
            "tcp:0.0.0.0:3001",
            "0.0.0.0",
            3001,
            Exposure::Public,
            vec![EntityRef::Process(key)],
        ));
        let digest = build_digest(&snapshot);
        assert_eq!(digest.exposed.total_public, 2);
        assert_eq!(digest.exposed.rows.len(), 1);
        assert_eq!(digest.exposed.rows[0].extra_ports, 1);
    }

    #[test]
    fn exposed_ranking_unowned_before_owned_before_known_system() {
        let mut snapshot = Snapshot::empty();
        snapshot.listeners.push(listener(
            "tcp:0.0.0.0:4000",
            "0.0.0.0",
            4000,
            Exposure::Public,
            vec![EntityRef::Process(process_key(2))],
        ));
        snapshot.listeners.push(listener(
            "tcp:0.0.0.0:3000",
            "0.0.0.0",
            3000,
            Exposure::Public,
            vec![],
        ));
        let digest = build_digest(&snapshot);
        assert_eq!(digest.exposed.rows[0].port, Some(3000));
        assert!(digest.exposed.rows[0].unowned);
    }

    #[test]
    fn digest_busy_snapshot_matches_golden_shape() {
        let snapshot: Snapshot =
            serde_json::from_str(include_str!("../../../../testdata/snapshots/busy.json"))
                .expect("busy snapshot fixture parses");
        let digest = build_digest(&snapshot);
        assert_eq!(digest.exposed.total_public, 2);
        assert_eq!(digest.exposed.total_lan, 1);
        assert_eq!(
            digest.exposed.rows.len(),
            2,
            "same owner public ports fold together"
        );
        assert_eq!(
            digest.exposed.rows[0].port,
            Some(5432),
            "unowned LAN port ranks first"
        );
        assert_eq!(digest.exposed.rows[1].extra_ports, 1);
        assert_eq!(digest.conflicts.total, 1);
        assert_eq!(digest.your_projects.total, 1);
        assert_eq!(digest.triage.summary.actionable, 2);
    }

    #[test]
    fn risk_rank_orders_worst_case_first() {
        // Lower number = higher priority. Unowned+public is worst.
        assert_eq!(risk_rank(true, true, false), 0);
        assert_eq!(risk_rank(true, false, false), 1);
        assert_eq!(risk_rank(false, true, true), 2);
        assert_eq!(risk_rank(false, true, false), 3);
        assert_eq!(risk_rank(false, false, true), 4);
        assert_eq!(risk_rank(false, false, false), 5);
        // Ordering is monotonic: each subsequent case is >= the previous.
        let ranks = [
            risk_rank(true, true, true),
            risk_rank(true, false, true),
            risk_rank(false, true, true),
            risk_rank(false, true, false),
            risk_rank(false, false, true),
            risk_rank(false, false, false),
        ];
        for w in ranks.windows(2) {
            assert!(w[0] <= w[1]);
        }
    }

    #[test]
    fn severity_rank_puts_errors_first() {
        assert!(severity_rank(&WarningSeverity::Error) < severity_rank(&WarningSeverity::Warning));
        assert!(severity_rank(&WarningSeverity::Warning) < severity_rank(&WarningSeverity::Info));
    }

    #[test]
    fn max_time_returns_later_or_only_available_value() {
        let earlier = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
        let later = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        assert_eq!(max_time(Some(earlier), Some(later)), Some(later));
        assert_eq!(max_time(Some(later), Some(earlier)), Some(later));
        assert_eq!(max_time(Some(later), None), Some(later));
        assert_eq!(max_time(None, Some(later)), Some(later));
        assert_eq!(max_time(None, None), None);
    }

    #[test]
    fn listener_bind_uses_unix_path_when_present() {
        let mut l = listener(
            "unix:/tmp/x.sock",
            "127.0.0.1",
            0,
            Exposure::UnixLocal,
            vec![],
        );
        l.path = Some(std::path::PathBuf::from("/tmp/x.sock"));
        let snapshot = Snapshot::empty();
        let relations = SnapshotRelations::new(&snapshot);
        assert_eq!(relations.listener_bind_label(&l), "/tmp/x.sock");
    }

    #[test]
    fn listener_bind_formats_host_and_port() {
        let l = listener("tcp:1.2.3.4:80", "1.2.3.4", 80, Exposure::Public, vec![]);
        let snapshot = Snapshot::empty();
        let relations = SnapshotRelations::new(&snapshot);
        assert_eq!(relations.listener_bind_label(&l), "1.2.3.4:80");
    }

    #[test]
    fn listener_bind_falls_back_to_star_when_no_addr() {
        let mut l = listener("tcp:*:80", "1.2.3.4", 80, Exposure::Public, vec![]);
        l.bind_addr = None;
        let snapshot = Snapshot::empty();
        let relations = SnapshotRelations::new(&snapshot);
        assert_eq!(relations.listener_bind_label(&l), "*:80");
    }

    #[test]
    fn owner_label_no_owner_renders_em_dash() {
        let snapshot = Snapshot::empty();
        let relations = SnapshotRelations::new(&snapshot);
        let l = listener("tcp:0:0", "0.0.0.0", 0, Exposure::Public, vec![]);
        assert_eq!(relations.listener_owner_label(&l, "\u{2014}"), "\u{2014}");
    }

    #[test]
    fn owner_label_uses_process_pid() {
        let snapshot = Snapshot::empty();
        let relations = SnapshotRelations::new(&snapshot);
        let l = listener(
            "tcp:0:0",
            "0.0.0.0",
            0,
            Exposure::Public,
            vec![EntityRef::Process(process_key(7))],
        );
        assert_eq!(relations.listener_owner_label(&l, "\u{2014}"), "pid 7");
    }

    #[test]
    fn owner_pid_resolves_through_workload_pids() {
        let mut snapshot = Snapshot::empty();
        let wid = WorkloadId::new("w-a");
        snapshot.workloads.push(Workload {
            id: wid.clone(),
            display_name: "w".into(),
            runtime: lazyadmin_core::model::RuntimeKind::Direct,
            state: WorkloadState::Running,
            pids: vec![process_key(42)],
            listeners: vec![],
            project: None,
            manager: None,
            source: None,
            actions: vec![],
            health: None,
            metrics: None,
            restart_policy: None,
            lazyadmin_run_id: None,
            provenance: vec![],
        });
        let l = listener(
            "tcp:0:0",
            "0.0.0.0",
            0,
            Exposure::Public,
            vec![EntityRef::Workload(wid)],
        );
        let relations = SnapshotRelations::new(&snapshot);
        assert_eq!(relations.listener_owner_pid(&l), Some(42));
    }

    #[test]
    fn digest_caps_each_section_at_limits() {
        let mut snapshot = Snapshot::empty();
        for port in 10_000..10_012 {
            snapshot.listeners.push(listener(
                format!("tcp:0.0.0.0:{port}"),
                "0.0.0.0",
                port,
                Exposure::Public,
                vec![],
            ));
        }
        for port in 20_000..20_007 {
            let id = format!("tcp:127.0.0.1:{port}");
            snapshot.listeners.push(listener(
                id.clone(),
                "127.0.0.1",
                port,
                Exposure::Loopback,
                vec![EntityRef::Process(process_key(i32::from(port)))],
            ));
            snapshot.warnings.push(lazyadmin_core::model::Warning {
                severity: WarningSeverity::Warning,
                code: "CONFLICT".into(),
                message: "conflict".into(),
                entity: Some(EntityRef::Listener(ListenerId::new(id))),
                provenance: vec![],
            });
        }
        for index in 0..12 {
            add_project_with_listener(&mut snapshot, index);
        }
        let digest = build_digest(&snapshot);
        assert_eq!(digest.exposed.rows.len(), DIGEST_EXPOSED_LIMIT);
        assert_eq!(digest.conflicts.rows.len(), DIGEST_CONFLICTS_LIMIT);
        assert_eq!(digest.your_projects.rows.len(), DIGEST_PROJECTS_LIMIT);
    }

    fn listener(
        id: impl Into<String>,
        addr: &str,
        port: u16,
        exposure: Exposure,
        owners: Vec<EntityRef>,
    ) -> Listener {
        let now = Utc.with_ymd_and_hms(2026, 4, 30, 12, 0, 0).unwrap();
        Listener {
            id: ListenerId::new(id),
            protocol: Protocol::Tcp,
            family: AddressFamily::Ipv4,
            bind_addr: Some(addr.to_string()),
            port: Some(port),
            path: None,
            state: ListenerState::Listen,
            netns: "host".into(),
            socket_inode: None,
            exposure,
            owners,
            confidence: Confidence::High,
            provenance: vec![],
            first_seen: now,
            last_seen: now,
            dual_stack_state: lazyadmin_core::model::DualStackState::Unknown,
        }
    }

    fn process_key(pid: i32) -> ProcessKey {
        ProcessKey {
            pid,
            boot_id: "boot".into(),
            start_time_ticks: pid as u64,
        }
    }

    fn add_project_with_listener(snapshot: &mut Snapshot, index: u16) {
        let project_id = ProjectId::new(format!("project:{index}"));
        let workload_id = WorkloadId::new(format!("workload:{index}"));
        let listener_id = ListenerId::new(format!("tcp:127.0.0.1:{}", 30_000 + index));
        snapshot.projects.push(Project {
            id: project_id.clone(),
            root: format!("/tmp/project-{index}").into(),
            name: format!("project-{index}"),
            markers: vec![],
            git_remote: None,
            package_manager: None,
            dev_commands: vec![],
            provenance: vec![],
        });
        snapshot.listeners.push(listener(
            listener_id.to_string(),
            "127.0.0.1",
            30_000 + index,
            Exposure::Loopback,
            vec![EntityRef::Workload(workload_id.clone())],
        ));
        snapshot.workloads.push(Workload {
            id: workload_id,
            display_name: format!("workload-{index}"),
            runtime: lazyadmin_core::model::RuntimeKind::Direct,
            state: WorkloadState::Running,
            pids: vec![],
            listeners: vec![listener_id],
            project: Some(project_id),
            manager: None,
            source: None,
            actions: vec![],
            health: None,
            metrics: None,
            restart_policy: None,
            lazyadmin_run_id: None,
            provenance: vec![],
        });
    }
}
