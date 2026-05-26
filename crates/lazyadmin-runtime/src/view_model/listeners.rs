use std::{cmp::Ordering, collections::HashSet};

use fuzzy_matcher::{FuzzyMatcher, skim::SkimMatcherV2};
use lazyadmin_core::{
    model::{
        Confidence, EntityRef, Exposure, Listener, ListenerId, Protocol, RuntimeKind, Snapshot,
    },
    output,
};
use serde::{Deserialize, Serialize};

use super::relations::SnapshotRelations;

pub const LISTENER_TABLE_SCHEMA_VERSION: &str = "lazyadmin.listener_table.v1";

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListenerTable {
    pub schema_version: String,
    pub rows: Vec<ListenerTableRow>,
    pub total: usize,
    pub returned: usize,
    pub filter: ListenerTableFilter,
    pub sort: ListenerTableSort,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListenerTableRow {
    pub id: String,
    pub port: Option<u16>,
    pub bind_label: String,
    pub endpoint_label: String,
    pub protocol: Protocol,
    pub protocol_label: String,
    pub exposure: Exposure,
    pub exposure_label: String,
    pub owner_label: String,
    pub runtime_label: String,
    pub project_label: Option<String>,
    pub confidence: Confidence,
    pub warning_count: usize,
    pub is_conflict: bool,
    pub is_orphan: bool,
    pub is_tracked: bool,
    pub is_project: bool,
    pub is_system: bool,
    pub signal: ListenerSignal,
    pub marker: ListenerMarker,
    pub search_text: String,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListenerTableFilter {
    #[default]
    All,
    Public,
    Lan,
    Conflicts,
    Orphans,
    Unowned,
    Tracked,
}

impl ListenerTableFilter {
    pub fn parse(value: &str) -> Self {
        match value {
            "public" => Self::Public,
            "lan" => Self::Lan,
            "conflicts" => Self::Conflicts,
            "orphans" => Self::Orphans,
            "unowned" => Self::Unowned,
            "tracked" => Self::Tracked,
            _ => Self::All,
        }
    }

    pub fn id(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Public => "public",
            Self::Lan => "lan",
            Self::Conflicts => "conflicts",
            Self::Orphans => "orphans",
            Self::Unowned => "unowned",
            Self::Tracked => "tracked",
        }
    }

    fn matches(self, row: &ListenerTableRow) -> bool {
        match self {
            Self::All => true,
            Self::Public => matches!(row.exposure, Exposure::Public | Exposure::LanOrPublic),
            Self::Lan => matches!(row.exposure, Exposure::LanOrPublic),
            Self::Conflicts => row.is_conflict,
            Self::Orphans | Self::Unowned => row.is_orphan,
            Self::Tracked => row.is_tracked,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListenerTableSortColumn {
    #[default]
    Port,
    Bind,
    Owner,
    Runtime,
    Exposure,
    Project,
    Confidence,
    Warnings,
}

impl ListenerTableSortColumn {
    pub fn parse(value: &str) -> Self {
        match value {
            "bind" => Self::Bind,
            "owner" => Self::Owner,
            "runtime" => Self::Runtime,
            "exposure" | "scope" => Self::Exposure,
            "project" => Self::Project,
            "confidence" => Self::Confidence,
            "warnings" => Self::Warnings,
            _ => Self::Port,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListenerTableSortDirection {
    #[default]
    Asc,
    Desc,
}

impl ListenerTableSortDirection {
    pub fn parse(value: &str) -> Self {
        if value == "desc" {
            Self::Desc
        } else {
            Self::Asc
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListenerTableSort {
    pub column: ListenerTableSortColumn,
    pub direction: ListenerTableSortDirection,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListenerSignal {
    Public,
    Lan,
    #[default]
    Local,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ListenerMarker {
    Conflict,
    Tracked,
    Project,
    #[default]
    None,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ListenerTableOptions {
    pub filter: ListenerTableFilter,
    pub sort: ListenerTableSort,
    pub show_system: bool,
    pub text_filter: String,
    pub text_match: ListenerTableTextMatch,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ListenerTableTextMatch {
    #[default]
    Substring,
    Fuzzy,
}

pub fn build_listener_table(snapshot: &Snapshot, options: ListenerTableOptions) -> ListenerTable {
    let mut rows = listener_table_rows(snapshot);
    let total = rows.len();
    rows.retain(|row| options.show_system || !row.is_system);
    rows.retain(|row| options.filter.matches(row));
    if !options.text_filter.trim().is_empty() {
        let needle = options.text_filter.to_ascii_lowercase();
        match options.text_match {
            ListenerTableTextMatch::Substring => {
                rows.retain(|row| row.search_text.contains(&needle));
            }
            ListenerTableTextMatch::Fuzzy => {
                let matcher = SkimMatcherV2::default();
                rows.retain(|row| matcher.fuzzy_match(&row.search_text, &needle).is_some());
            }
        }
    }
    sort_listener_table_rows(&mut rows, options.sort);
    ListenerTable {
        schema_version: LISTENER_TABLE_SCHEMA_VERSION.to_string(),
        returned: rows.len(),
        rows,
        total,
        filter: options.filter,
        sort: options.sort,
    }
}

pub fn listener_table_rows(snapshot: &Snapshot) -> Vec<ListenerTableRow> {
    let relations = SnapshotRelations::new(snapshot);
    let conflict_ids: HashSet<ListenerId> = snapshot
        .warnings
        .iter()
        .filter(|warning| warning.code == "CONFLICT")
        .filter_map(|warning| match &warning.entity {
            Some(EntityRef::Listener(id)) => Some(id.clone()),
            _ => None,
        })
        .collect();
    let projected = output::listener_rows(snapshot);
    snapshot
        .listeners
        .iter()
        .map(|listener| {
            let projected = projected.iter().find(|row| row.id == listener.id);
            listener_table_row(snapshot, &relations, &conflict_ids, listener, projected)
        })
        .collect()
}

pub fn sort_listener_table_rows(rows: &mut [ListenerTableRow], sort: ListenerTableSort) {
    rows.sort_by(|a, b| {
        let ord = match sort.column {
            ListenerTableSortColumn::Port => compare_port(a, b),
            ListenerTableSortColumn::Bind => a.bind_label.cmp(&b.bind_label),
            ListenerTableSortColumn::Owner => a.owner_label.cmp(&b.owner_label),
            ListenerTableSortColumn::Runtime => a.runtime_label.cmp(&b.runtime_label),
            ListenerTableSortColumn::Exposure => a.exposure_label.cmp(&b.exposure_label),
            ListenerTableSortColumn::Project => a.project_label.cmp(&b.project_label),
            ListenerTableSortColumn::Confidence => {
                format!("{:?}", a.confidence).cmp(&format!("{:?}", b.confidence))
            }
            ListenerTableSortColumn::Warnings => a.warning_count.cmp(&b.warning_count),
        }
        .then_with(|| a.id.cmp(&b.id));
        if sort.direction == ListenerTableSortDirection::Desc {
            ord.reverse()
        } else {
            ord
        }
    });
}

fn listener_table_row(
    snapshot: &Snapshot,
    relations: &SnapshotRelations<'_>,
    conflict_ids: &HashSet<ListenerId>,
    listener: &Listener,
    projected: Option<&output::ListenerRow>,
) -> ListenerTableRow {
    let bind_label = listener_bind_label(listener);
    let endpoint_label = listener_endpoint_label(listener);
    let is_conflict = conflict_ids.contains(&listener.id) || listener.owners.len() > 1;
    let is_orphan = listener.owners.is_empty();
    let is_tracked = listener_is_tracked(listener, snapshot);
    let is_project = listener_is_project_member(listener, snapshot);
    let is_system = listener
        .provenance
        .iter()
        .any(|provenance| provenance.claim.contains("systemd:system"))
        || relations.is_system_listener(listener);
    let owner_label = projected
        .and_then(|row| row.manager_detail.clone())
        .unwrap_or_else(|| listener_owner_label(listener, snapshot, relations));
    let runtime_label = projected
        .and_then(|row| row.manager_label.clone())
        .unwrap_or_else(|| listener_runtime_label(listener, snapshot, relations, is_system));
    let project_label = relations.listener_project_label(listener);
    let warning_count = snapshot
        .warnings
        .iter()
        .filter(|warning| matches!(&warning.entity, Some(EntityRef::Listener(id)) if id == &listener.id))
        .count();
    let exposure_label = exposure_label(&listener.exposure);
    let protocol_label = format!("{:?}", listener.protocol).to_ascii_lowercase();
    let signal = match listener.exposure {
        Exposure::Public => ListenerSignal::Public,
        Exposure::LanOrPublic => ListenerSignal::Lan,
        _ => ListenerSignal::Local,
    };
    let marker = if is_conflict {
        ListenerMarker::Conflict
    } else if is_tracked {
        ListenerMarker::Tracked
    } else if is_project {
        ListenerMarker::Project
    } else {
        ListenerMarker::None
    };
    let search_text = [
        listener.id.to_string(),
        listener
            .port
            .map(|port| port.to_string())
            .unwrap_or_default(),
        bind_label.clone(),
        endpoint_label.clone(),
        owner_label.clone(),
        runtime_label.clone(),
        exposure_label.clone(),
        project_label.clone().unwrap_or_default(),
        protocol_label.clone(),
        format!("{:?}", listener.confidence).to_ascii_lowercase(),
    ]
    .join(" ")
    .to_ascii_lowercase();

    ListenerTableRow {
        id: listener.id.to_string(),
        port: listener.port,
        bind_label,
        endpoint_label,
        protocol: listener.protocol.clone(),
        protocol_label,
        exposure: listener.exposure.clone(),
        exposure_label,
        owner_label,
        runtime_label,
        project_label,
        confidence: listener.confidence,
        warning_count,
        is_conflict,
        is_orphan,
        is_tracked,
        is_project,
        is_system,
        signal,
        marker,
        search_text,
    }
}

fn compare_port(a: &ListenerTableRow, b: &ListenerTableRow) -> Ordering {
    match (a.port, b.port) {
        (Some(ap), Some(bp)) => ap.cmp(&bp),
        (Some(_), None) => Ordering::Less,
        (None, Some(_)) => Ordering::Greater,
        (None, None) => Ordering::Equal,
    }
}

fn listener_bind_label(listener: &Listener) -> String {
    if let Some(path) = &listener.path {
        return path.display().to_string();
    }
    listener
        .bind_addr
        .clone()
        .unwrap_or_else(|| "*".to_string())
}

fn listener_endpoint_label(listener: &Listener) -> String {
    if let Some(path) = &listener.path {
        return path.display().to_string();
    }
    match listener.port {
        Some(port) => format!("{}:{port}", listener_bind_label(listener)),
        None => listener_bind_label(listener),
    }
}

fn listener_is_tracked(listener: &Listener, snapshot: &Snapshot) -> bool {
    listener.owners.iter().any(|owner| match owner {
        EntityRef::Run(_) => true,
        EntityRef::Process(key) => snapshot
            .processes
            .iter()
            .any(|process| &process.key == key && process.lazyadmin_run_id.is_some()),
        EntityRef::Workload(id) => snapshot
            .workloads
            .iter()
            .any(|workload| &workload.id == id && workload.lazyadmin_run_id.is_some()),
        _ => false,
    }) || snapshot.workloads.iter().any(|workload| {
        workload.lazyadmin_run_id.is_some()
            && workload.listeners.iter().any(|id| id == &listener.id)
    })
}

fn listener_is_project_member(listener: &Listener, snapshot: &Snapshot) -> bool {
    if listener
        .owners
        .iter()
        .any(|owner| matches!(owner, EntityRef::Project(_)))
    {
        return true;
    }
    snapshot.workloads.iter().any(|workload| {
        workload.project.is_some()
            && (workload.listeners.iter().any(|id| id == &listener.id)
                || listener
                    .owners
                    .iter()
                    .any(|owner| matches!(owner, EntityRef::Workload(id) if id == &workload.id))
                || listener.owners.iter().any(|owner| match owner {
                    EntityRef::Process(key) => workload.pids.iter().any(|pid| pid == key),
                    _ => false,
                }))
    })
}

fn listener_owner_label(
    listener: &Listener,
    snapshot: &Snapshot,
    relations: &SnapshotRelations<'_>,
) -> String {
    listener
        .owners
        .iter()
        .find_map(|owner| match owner {
            EntityRef::Workload(id) => relations
                .workload(id)
                .map(|workload| compact_text(&workload.display_name, 38)),
            EntityRef::Process(key) => relations
                .process(key)
                .map(process_owner_label)
                .or_else(|| Some(format!("pid {}", key.pid))),
            EntityRef::Manager(id) => relations
                .manager(id)
                .map(|manager| compact_text(&manager.name, 38)),
            EntityRef::Project(id) => relations
                .project(id)
                .map(|project| compact_text(&project.name, 38)),
            EntityRef::Run(id) => Some(format!("run {}", short_id(&id.to_string()))),
            EntityRef::Listener(id) => Some(format!("listener {}", short_id(&id.to_string()))),
            EntityRef::Action(id) => Some(format!("action {}", short_id(&id.to_string()))),
        })
        .or_else(|| {
            snapshot
                .workloads
                .iter()
                .find(|workload| workload.listeners.iter().any(|id| id == &listener.id))
                .map(|workload| compact_text(&workload.display_name, 38))
        })
        .unwrap_or_else(|| "unowned".into())
}

fn listener_runtime_label(
    listener: &Listener,
    snapshot: &Snapshot,
    relations: &SnapshotRelations<'_>,
    is_system: bool,
) -> String {
    if let Some(label) = listener.owners.iter().find_map(|owner| match owner {
        EntityRef::Workload(id) => relations
            .workload(id)
            .map(|workload| runtime_kind_label(&workload.runtime)),
        EntityRef::Manager(id) => relations
            .manager(id)
            .map(|manager| runtime_kind_label(&manager.kind)),
        EntityRef::Process(key) => relations.process(key).map(process_runtime_label),
        _ => None,
    }) {
        return label;
    }
    if let Some(label) = snapshot
        .workloads
        .iter()
        .find(|workload| workload.listeners.iter().any(|id| id == &listener.id))
        .map(|workload| runtime_kind_label(&workload.runtime))
    {
        return label;
    }
    if is_system {
        "systemd".into()
    } else {
        "direct".into()
    }
}

fn process_owner_label(process: &lazyadmin_core::model::Process) -> String {
    let command = process
        .exe
        .as_ref()
        .and_then(|path| path.file_name())
        .and_then(|name| name.to_str())
        .map(ToString::to_string)
        .or_else(|| {
            process.cmdline.first().map(|cmd| {
                std::path::Path::new(cmd)
                    .file_name()
                    .and_then(|name| name.to_str())
                    .unwrap_or(cmd)
                    .to_string()
            })
        })
        .filter(|cmd| !cmd.trim().is_empty())
        .unwrap_or_else(|| "process".into());
    compact_text(&format!("{command} pid {}", process.pid), 38)
}

fn process_runtime_label(process: &lazyadmin_core::model::Process) -> String {
    if process.systemd_unit.is_some() {
        "systemd".into()
    } else if process.container_id.is_some() {
        "container".into()
    } else if process.lazyadmin_run_id.is_some() {
        "tracked".into()
    } else {
        "direct".into()
    }
}

fn runtime_kind_label(kind: &RuntimeKind) -> String {
    match kind {
        RuntimeKind::Direct => "direct",
        RuntimeKind::LazyadminTracked => "tracked",
        RuntimeKind::SystemdSystem | RuntimeKind::SystemdUser | RuntimeKind::SystemdSocket => {
            "systemd"
        }
        RuntimeKind::Docker => "docker",
        RuntimeKind::DockerCompose => "compose",
        RuntimeKind::Portless => "portless",
        RuntimeKind::Podman => "podman",
        RuntimeKind::PodmanCompose => "podman-compose",
        RuntimeKind::PodmanPod => "podman-pod",
        RuntimeKind::KubectlPortForward => "kubectl",
        RuntimeKind::SshTunnel => "ssh",
        RuntimeKind::Cloudflared => "cloudflared",
        RuntimeKind::Socat => "socat",
        RuntimeKind::Supervisor => "supervisor",
        RuntimeKind::Launchd => "launchd",
        RuntimeKind::Unknown => "unknown",
    }
    .into()
}

fn exposure_label(exposure: &Exposure) -> String {
    match exposure {
        Exposure::Loopback => "loopback",
        Exposure::LanOrPublic => "lan",
        Exposure::Public => "public",
        Exposure::ContainerOnly => "container",
        Exposure::UnixLocal => "unix",
        Exposure::Unknown => "unknown",
    }
    .into()
}

fn short_id(value: &str) -> String {
    compact_text(value, 12)
}

fn compact_text(value: &str, max_chars: usize) -> String {
    if value.chars().count() <= max_chars {
        return value.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    format!("{}…", value.chars().take(keep).collect::<String>())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn empty_snapshot() -> Snapshot {
        serde_json::from_str(include_str!("../../../../testdata/snapshots/empty.json")).unwrap()
    }

    fn busy_snapshot() -> Snapshot {
        serde_json::from_str(include_str!("../../../../testdata/snapshots/busy.json")).unwrap()
    }

    #[test]
    fn empty_snapshot_projects_empty_listener_table() {
        let table = build_listener_table(&empty_snapshot(), ListenerTableOptions::default());
        assert_eq!(table.schema_version, LISTENER_TABLE_SCHEMA_VERSION);
        assert_eq!(table.total, 0);
        assert!(table.rows.is_empty());
    }

    #[test]
    fn busy_snapshot_projects_listener_facts() {
        let table = build_listener_table(&busy_snapshot(), ListenerTableOptions::default());
        let row = table
            .rows
            .iter()
            .find(|row| row.id == "tcp:127.0.0.1:8080")
            .expect("api listener row");

        assert_eq!(row.port, Some(8080));
        assert_eq!(row.bind_label, "127.0.0.1");
        assert_eq!(row.endpoint_label, "127.0.0.1:8080");
        assert_eq!(row.owner_label, "api dev server");
        assert_eq!(row.runtime_label, "direct");
        assert_eq!(row.project_label.as_deref(), Some("api"));
        assert!(row.is_project);
        assert!(row.is_conflict);
        assert_eq!(row.warning_count, 1);
        assert!(!row.is_orphan);
    }

    #[test]
    fn filters_apply_shared_listener_predicates() {
        let snapshot = busy_snapshot();
        let public = build_listener_table(
            &snapshot,
            ListenerTableOptions {
                filter: ListenerTableFilter::Public,
                ..ListenerTableOptions::default()
            },
        );
        assert!(
            public
                .rows
                .iter()
                .all(|row| matches!(row.exposure, Exposure::Public | Exposure::LanOrPublic))
        );

        let conflicts = build_listener_table(
            &snapshot,
            ListenerTableOptions {
                filter: ListenerTableFilter::Conflicts,
                ..ListenerTableOptions::default()
            },
        );
        assert!(conflicts.rows.iter().all(|row| row.is_conflict));
    }

    #[test]
    fn sort_orders_ports_with_missing_ports_last() {
        let mut rows = listener_table_rows(&busy_snapshot());
        sort_listener_table_rows(
            &mut rows,
            ListenerTableSort {
                column: ListenerTableSortColumn::Port,
                direction: ListenerTableSortDirection::Asc,
            },
        );

        let ports: Vec<_> = rows.iter().filter_map(|row| row.port).collect();
        let mut sorted_ports = ports.clone();
        sorted_ports.sort();
        assert_eq!(ports, sorted_ports);
    }
}
