//! Per-entity-kind inspector view-models. PLAN-15d / Issue #17.
//!
//! These view-models are the *richest* surface in either UI: full
//! identifiers (never truncated), zero `-` filler rows, related
//! entities listed inline, action commands previewed before
//! confirmation. The TUI and Web inspector both consume the same
//! struct so the two surfaces can never drift section-by-section.
//!
//! Rendering contract:
//! - Missing data is *omitted*, not rendered as `-` or `unavailable`.
//! - Any text that names an entity is the **full** value (no
//!   `tcp:127.0.0…` ellipsis).
//! - Each variant exposes both typed fields *and* a `to_sections()`
//!   helper that flattens the variant into a sequence of
//!   [`InspectorSection`]s. Renderers walk the section list and stay
//!   kind-agnostic.

use std::collections::HashMap;

use lazyadmin_core::model::*;
use serde::{Deserialize, Serialize};

/// Top-level dispatch. Each variant is the typed view-model for one
/// entity kind.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InspectorView {
    Listener(ListenerInspector),
    Workload(WorkloadInspector),
    Process(ProcessInspector),
    Project(ProjectInspector),
    Manager(ManagerInspector),
    TrackedRun(TrackedRunInspector),
    WarningGroup(WarningGroupInspector),
}

/// Stable section identifiers used by the rendered output. The
/// renderer relies on this list, not on the variant — that's how we
/// guarantee TUI and Web stay in lockstep.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectorSection {
    pub heading: &'static str,
    pub rows: Vec<InspectorRow>,
}

/// One row of section content. `value` may wrap across lines but is
/// never truncated. `secondary` is an optional dim hint shown after
/// the value (file path, count suffix, etc.).
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct InspectorRow {
    pub label: String,
    pub value: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub secondary: Option<String>,
    /// When set, the row is itself a navigation target (related
    /// listener / process / project). The renderer can attach a
    /// one-key shortcut.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump_target: Option<JumpTarget>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum JumpTarget {
    Listener { id: ListenerId },
    Workload { id: WorkloadId },
    Process { key: ProcessKey },
    Project { id: ProjectId },
    Manager { id: ManagerId },
    TrackedRun { id: RunId },
    WarningGroup { code: String },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ConfidenceSignal {
    /// procfs `/proc/<pid>/fd/*` resolved to a socket inode.
    ProcfsPidInode,
    /// Container-runtime metadata (Docker engine API / Podman).
    ContainerInspect,
    /// systemd unit / cgroup correlation.
    CgroupCorrelation,
    /// Manager attribution heuristic (workload name pattern, etc.).
    ManagerAttribution,
    /// Tracked-run registry was authoritative.
    TrackedRunRegistry,
    /// Portless route file mapping a port to a project.
    PortlessRoutes,
    /// Best-effort fallback. The pessimistic case.
    BestEffort,
}

impl ConfidenceSignal {
    pub fn label(&self) -> &'static str {
        match self {
            Self::ProcfsPidInode => "procfs PID→socket inode",
            Self::ContainerInspect => "container runtime inspect",
            Self::CgroupCorrelation => "systemd cgroup correlation",
            Self::ManagerAttribution => "manager attribution heuristic",
            Self::TrackedRunRegistry => "tracked-run registry",
            Self::PortlessRoutes => "portless routes file",
            Self::BestEffort => "best-effort fallback",
        }
    }

    /// Best-guess classification of a [`Provenance`] adapter name.
    /// Conservative: anything we don't recognize falls through to
    /// `BestEffort` so the user sees the truthful "this is a guess"
    /// label rather than a confident wrong one.
    pub fn classify(adapter: &str) -> Self {
        let lower = adapter.to_lowercase();
        if lower.contains("procfs") {
            Self::ProcfsPidInode
        } else if lower.contains("container")
            || lower.contains("docker")
            || lower.contains("podman")
        {
            Self::ContainerInspect
        } else if lower.contains("systemd") || lower.contains("cgroup") {
            Self::CgroupCorrelation
        } else if lower.contains("tracked") {
            Self::TrackedRunRegistry
        } else if lower.contains("portless") {
            Self::PortlessRoutes
        } else if lower.contains("project") || lower.contains("manager") {
            Self::ManagerAttribution
        } else {
            Self::BestEffort
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfidenceBlock {
    pub value: Confidence,
    pub signals: Vec<ConfidenceSignalEntry>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConfidenceSignalEntry {
    pub signal: ConfidenceSignal,
    pub adapter: String,
    pub claim: String,
}

/// One previewed action: the verb the operator can invoke, the
/// keybind hint, and the *exact* command lazyadmin will run before
/// the typed-verb confirmation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionPreview {
    pub verb: String,
    pub key_hint: String,
    pub command_string: String,
    pub enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub disabled_reason: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WarningRef {
    pub code: String,
    pub severity: WarningSeverity,
    pub message: String,
}

// ─── per-kind shapes ───────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListenerInspector {
    pub id: ListenerId,
    pub title: String,
    pub identity: ListenerIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub process: Option<ProcessFragment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub related_listeners: Vec<RelatedListener>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectRefBlock>,
    pub confidence: ConfidenceBlock,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ActionPreview>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<WarningRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ListenerIdentity {
    pub listener_id: String,
    pub bind: String,
    pub protocol: Protocol,
    pub family: AddressFamily,
    pub exposure: Exposure,
    pub state: ListenerState,
    pub netns: NamespaceId,
    pub owner_label: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessFragment {
    pub pid: i32,
    pub cmdline_full: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub exe: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub parent_pid: Option<i32>,
    /// Children PIDs of this process. Populated only when the snapshot
    /// has the data; capped at 32 entries to stay readable.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub children: Vec<i32>,
    pub key: ProcessKey,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RelatedListener {
    pub listener_id: ListenerId,
    pub bind: String,
    pub exposure: Exposure,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectRefBlock {
    pub project_id: ProjectId,
    pub name: String,
    pub root: String,
}

// ── workload ─────────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadInspector {
    pub id: WorkloadId,
    pub title: String,
    pub identity: WorkloadIdentity,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub child_processes: Vec<ProcessFragment>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub listeners: Vec<RelatedListener>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub project: Option<ProjectRefBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub manager: Option<ManagerRefBlock>,
    pub confidence: ConfidenceBlock,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ActionPreview>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<WarningRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadIdentity {
    pub workload_id: String,
    pub display_name: String,
    pub runtime: RuntimeKind,
    pub state: WorkloadState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub health: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub restart_policy: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagerRefBlock {
    pub manager_id: ManagerId,
    pub name: String,
    pub kind: RuntimeKind,
}

// ── process ──────────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProcessInspector {
    pub key: ProcessKey,
    pub title: String,
    pub identity: ProcessFragment,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub listeners: Vec<RelatedListener>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload: Option<WorkloadRefBlock>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tracked_run: Option<TrackedRunRefBlock>,
    pub confidence: ConfidenceBlock,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ActionPreview>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub warnings: Vec<WarningRef>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkloadRefBlock {
    pub workload_id: WorkloadId,
    pub display_name: String,
    pub runtime: RuntimeKind,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackedRunRefBlock {
    pub run_id: RunId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
}

// ── project ──────────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectInspector {
    pub id: ProjectId,
    pub title: String,
    pub identity: ProjectIdentity,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub workloads: Vec<WorkloadRefBlock>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub listeners: Vec<RelatedListener>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub markers: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProjectIdentity {
    pub project_id: String,
    pub name: String,
    pub root: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub git_remote: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub package_manager: Option<String>,
}

// ── manager ──────────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagerInspector {
    pub id: ManagerId,
    pub title: String,
    pub identity: ManagerIdentity,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub managed_workloads: Vec<WorkloadRefBlock>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManagerIdentity {
    pub manager_id: String,
    pub name: String,
    pub kind: RuntimeKind,
    pub scope: ManagerScope,
    pub available: bool,
    pub permission: PermissionState,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub socket: Option<String>,
}

// ── tracked run ──────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackedRunInspector {
    pub id: RunId,
    pub title: String,
    pub identity: TrackedRunIdentity,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub workload: Option<WorkloadRefBlock>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<ActionPreview>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrackedRunIdentity {
    pub run_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tag: Option<String>,
    pub command: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    pub state: WorkloadState,
}

// ── warning group ────────────────────────────────────────────────
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WarningGroupInspector {
    pub code: String,
    pub title: String,
    pub label: String,
    pub remediation: String,
    pub tier: lazyadmin_core::doctor::WarningTier,
    pub severity: WarningSeverity,
    pub count: usize,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub sample_entities: Vec<EntityRef>,
}

// ─── builder ──────────────────────────────────────────────────────

impl InspectorView {
    /// Locate a single inspector view by `kind` + `id`. Used by the
    /// read-only Web UI's `/api/inspector` endpoint and the TUI when
    /// the operator selects a row. `kind` accepts both singular and
    /// plural spellings (`listener`/`listeners`). For processes,
    /// `id` may be either the JSON-encoded `ProcessKey` or the bare
    /// PID string. Warning groups look up by `code`.
    pub fn lookup(snapshot: &Snapshot, kind: &str, id: &str) -> Option<Self> {
        match kind {
            "listener" | "listeners" => snapshot
                .listeners
                .iter()
                .find(|listener| listener.id.to_string() == id)
                .map(|listener| Self::Listener(build_listener_inspector(listener, snapshot))),
            "workload" | "workloads" => snapshot
                .workloads
                .iter()
                .find(|workload| workload.id.to_string() == id)
                .map(|workload| Self::Workload(build_workload_inspector(workload, snapshot))),
            "process" | "processes" => snapshot
                .processes
                .iter()
                .find(|process| {
                    serde_json::to_string(&process.key)
                        .ok()
                        .is_some_and(|k| k == id)
                        || process.pid.to_string() == id
                })
                .map(|process| Self::Process(build_process_inspector(process, snapshot))),
            "project" | "projects" => snapshot
                .projects
                .iter()
                .find(|project| project.id.to_string() == id)
                .map(|project| Self::Project(build_project_inspector(project, snapshot))),
            "manager" | "managers" => snapshot
                .managers
                .iter()
                .find(|manager| manager.id.to_string() == id)
                .map(|manager| Self::Manager(build_manager_inspector(manager, snapshot))),
            "run" | "runs" | "tracked_run" | "tracked_runs" => snapshot
                .tracked_runs
                .iter()
                .find(|run| run.id.to_string() == id)
                .map(|run| Self::TrackedRun(build_tracked_run_inspector(run, snapshot))),
            "warning" | "warning_group" | "warning_groups" => {
                build_warning_group_inspector(snapshot, id).map(Self::WarningGroup)
            }
            _ => None,
        }
    }

    /// Build inspector views for every entity in `snapshot`. Used by
    /// callers that want to walk the full set (e.g. cache priming or
    /// snapshot tests).
    pub fn all_from_snapshot(snapshot: &Snapshot) -> Vec<Self> {
        let mut out = Vec::with_capacity(
            snapshot.listeners.len()
                + snapshot.workloads.len()
                + snapshot.processes.len()
                + snapshot.projects.len()
                + snapshot.managers.len()
                + snapshot.tracked_runs.len(),
        );
        for listener in &snapshot.listeners {
            out.push(Self::Listener(build_listener_inspector(listener, snapshot)));
        }
        for workload in &snapshot.workloads {
            out.push(Self::Workload(build_workload_inspector(workload, snapshot)));
        }
        for process in &snapshot.processes {
            out.push(Self::Process(build_process_inspector(process, snapshot)));
        }
        for project in &snapshot.projects {
            out.push(Self::Project(build_project_inspector(project, snapshot)));
        }
        for manager in &snapshot.managers {
            out.push(Self::Manager(build_manager_inspector(manager, snapshot)));
        }
        for run in &snapshot.tracked_runs {
            out.push(Self::TrackedRun(build_tracked_run_inspector(run, snapshot)));
        }
        out
    }

    /// Stable kind discriminator used by URL routing and the renderer.
    pub fn kind(&self) -> &'static str {
        match self {
            Self::Listener(_) => "listener",
            Self::Workload(_) => "workload",
            Self::Process(_) => "process",
            Self::Project(_) => "project",
            Self::Manager(_) => "manager",
            Self::TrackedRun(_) => "tracked_run",
            Self::WarningGroup(_) => "warning_group",
        }
    }

    /// Title shown above the inspector body.
    pub fn title(&self) -> &str {
        match self {
            Self::Listener(v) => &v.title,
            Self::Workload(v) => &v.title,
            Self::Process(v) => &v.title,
            Self::Project(v) => &v.title,
            Self::Manager(v) => &v.title,
            Self::TrackedRun(v) => &v.title,
            Self::WarningGroup(v) => &v.title,
        }
    }

    /// Flatten the variant into a sequence of [`InspectorSection`]s.
    /// The renderer is kind-agnostic.
    pub fn to_sections(&self) -> Vec<InspectorSection> {
        match self {
            Self::Listener(v) => listener_sections(v),
            Self::Workload(v) => workload_sections(v),
            Self::Process(v) => process_sections(v),
            Self::Project(v) => project_sections(v),
            Self::Manager(v) => manager_sections(v),
            Self::TrackedRun(v) => tracked_run_sections(v),
            Self::WarningGroup(v) => warning_group_sections(v),
        }
    }
}

// ─── builders (per-kind) ───────────────────────────────────────────

fn build_listener_inspector(listener: &Listener, snapshot: &Snapshot) -> ListenerInspector {
    let owner_pid = first_owner_pid(listener, snapshot);
    let process_fragment = owner_pid
        .and_then(|pid| snapshot.processes.iter().find(|p| p.pid == pid))
        .map(|p| process_fragment(p, snapshot));
    let related_listeners = owner_pid
        .map(|pid| listeners_for_pid(snapshot, pid, &listener.id))
        .unwrap_or_default();
    let project = listener_project(listener, snapshot);
    let warnings = warnings_for(
        snapshot,
        |w| matches!(&w.entity, Some(EntityRef::Listener(id)) if id == &listener.id),
    );
    let actions = listener_actions(listener);
    let confidence = build_confidence(listener.confidence, &listener.provenance);
    ListenerInspector {
        id: listener.id.clone(),
        title: listener_label(listener),
        identity: ListenerIdentity {
            listener_id: listener.id.to_string(),
            bind: listener_bind(listener),
            protocol: listener.protocol.clone(),
            family: listener.family.clone(),
            exposure: listener.exposure.clone(),
            state: listener.state.clone(),
            netns: listener.netns.clone(),
            owner_label: owner_label(listener, snapshot),
            user: process_fragment.as_ref().and_then(|p| p.user.clone()),
        },
        process: process_fragment,
        related_listeners,
        project,
        confidence,
        actions,
        warnings,
    }
}

fn build_workload_inspector(workload: &Workload, snapshot: &Snapshot) -> WorkloadInspector {
    let listeners = workload
        .listeners
        .iter()
        .filter_map(|listener_id| {
            snapshot
                .listeners
                .iter()
                .find(|listener| &listener.id == listener_id)
        })
        .map(|listener| RelatedListener {
            listener_id: listener.id.clone(),
            bind: listener_bind(listener),
            exposure: listener.exposure.clone(),
        })
        .collect();
    let child_processes = workload
        .pids
        .iter()
        .filter_map(|key| {
            snapshot
                .processes
                .iter()
                .find(|process| &process.key == key)
        })
        .map(|process| process_fragment(process, snapshot))
        .collect();
    let project = workload
        .project
        .as_ref()
        .and_then(|id| snapshot.projects.iter().find(|p| &p.id == id))
        .map(|project| ProjectRefBlock {
            project_id: project.id.clone(),
            name: project.name.clone(),
            root: project.root.display().to_string(),
        });
    let manager = workload
        .manager
        .as_ref()
        .and_then(|id| snapshot.managers.iter().find(|m| &m.id == id))
        .map(|manager| ManagerRefBlock {
            manager_id: manager.id.clone(),
            name: manager.name.clone(),
            kind: manager.kind.clone(),
        });
    let warnings = warnings_for(
        snapshot,
        |w| matches!(&w.entity, Some(EntityRef::Workload(id)) if id == &workload.id),
    );
    let actions = workload_actions(workload);
    let confidence = build_confidence(Confidence::Medium, &workload.provenance);
    WorkloadInspector {
        id: workload.id.clone(),
        title: workload.display_name.clone(),
        identity: WorkloadIdentity {
            workload_id: workload.id.to_string(),
            display_name: workload.display_name.clone(),
            runtime: workload.runtime.clone(),
            state: workload.state.clone(),
            health: workload.health.clone(),
            restart_policy: workload.restart_policy.as_ref().map(|p| p.policy.clone()),
        },
        child_processes,
        listeners,
        project,
        manager,
        confidence,
        actions,
        warnings,
    }
}

fn build_process_inspector(process: &Process, snapshot: &Snapshot) -> ProcessInspector {
    let identity = process_fragment(process, snapshot);
    let listeners = listeners_for_pid(snapshot, process.pid, &ListenerId::new(""));
    let workload = snapshot
        .workloads
        .iter()
        .find(|workload| workload.pids.iter().any(|key| key == &process.key))
        .map(|workload| WorkloadRefBlock {
            workload_id: workload.id.clone(),
            display_name: workload.display_name.clone(),
            runtime: workload.runtime.clone(),
        });
    let tracked_run = process.lazyadmin_run_id.as_ref().map(|run_id| {
        let tag = snapshot
            .tracked_runs
            .iter()
            .find(|run| &run.id == run_id)
            .and_then(|run| run.tag.clone());
        TrackedRunRefBlock {
            run_id: run_id.clone(),
            tag,
        }
    });
    let warnings = warnings_for(
        snapshot,
        |w| matches!(&w.entity, Some(EntityRef::Process(key)) if key == &process.key),
    );
    let actions = process_actions(process);
    let confidence = build_confidence(Confidence::High, &process.provenance);
    let title = process
        .cmdline
        .first()
        .cloned()
        .unwrap_or_else(|| format!("pid {}", process.pid));
    ProcessInspector {
        key: process.key.clone(),
        title,
        identity,
        listeners,
        workload,
        tracked_run,
        confidence,
        actions,
        warnings,
    }
}

fn build_project_inspector(project: &Project, snapshot: &Snapshot) -> ProjectInspector {
    let workloads: Vec<WorkloadRefBlock> = snapshot
        .workloads
        .iter()
        .filter(|workload| workload.project.as_ref() == Some(&project.id))
        .map(|workload| WorkloadRefBlock {
            workload_id: workload.id.clone(),
            display_name: workload.display_name.clone(),
            runtime: workload.runtime.clone(),
        })
        .collect();
    let mut listener_ids = std::collections::BTreeSet::new();
    for workload in snapshot
        .workloads
        .iter()
        .filter(|workload| workload.project.as_ref() == Some(&project.id))
    {
        for listener_id in &workload.listeners {
            listener_ids.insert(listener_id.clone());
        }
    }
    let listeners: Vec<RelatedListener> = listener_ids
        .into_iter()
        .filter_map(|id| snapshot.listeners.iter().find(|l| l.id == id))
        .map(|l| RelatedListener {
            listener_id: l.id.clone(),
            bind: listener_bind(l),
            exposure: l.exposure.clone(),
        })
        .collect();
    let markers = project
        .markers
        .iter()
        .map(|m| format!("{}: {}", m.kind, m.path.display()))
        .collect();
    ProjectInspector {
        id: project.id.clone(),
        title: project.name.clone(),
        identity: ProjectIdentity {
            project_id: project.id.to_string(),
            name: project.name.clone(),
            root: project.root.display().to_string(),
            git_remote: project.git_remote.clone(),
            package_manager: project.package_manager.clone(),
        },
        workloads,
        listeners,
        markers,
    }
}

fn build_manager_inspector(manager: &Manager, snapshot: &Snapshot) -> ManagerInspector {
    let managed_workloads = snapshot
        .workloads
        .iter()
        .filter(|workload| workload.manager.as_ref() == Some(&manager.id))
        .map(|workload| WorkloadRefBlock {
            workload_id: workload.id.clone(),
            display_name: workload.display_name.clone(),
            runtime: workload.runtime.clone(),
        })
        .collect();
    ManagerInspector {
        id: manager.id.clone(),
        title: manager.name.clone(),
        identity: ManagerIdentity {
            manager_id: manager.id.to_string(),
            name: manager.name.clone(),
            kind: manager.kind.clone(),
            scope: manager.scope.clone(),
            available: manager.available,
            permission: manager.permission.clone(),
            version: manager.version.clone(),
            socket: manager.socket.as_ref().map(|p| p.display().to_string()),
        },
        managed_workloads,
    }
}

fn build_tracked_run_inspector(run: &TrackedRun, snapshot: &Snapshot) -> TrackedRunInspector {
    let workload = snapshot
        .workloads
        .iter()
        .find(|workload| workload.lazyadmin_run_id.as_ref() == Some(&run.id))
        .map(|workload| WorkloadRefBlock {
            workload_id: workload.id.clone(),
            display_name: workload.display_name.clone(),
            runtime: workload.runtime.clone(),
        });
    let title = run.tag.clone().unwrap_or_else(|| run.command.join(" "));
    TrackedRunInspector {
        id: run.id.clone(),
        title: title.clone(),
        identity: TrackedRunIdentity {
            run_id: run.id.to_string(),
            tag: run.tag.clone(),
            command: run.command.join(" "),
            cwd: run.cwd.as_ref().map(|p| p.display().to_string()),
            state: run.state.clone(),
        },
        workload,
        actions: tracked_run_actions(run),
    }
}

fn build_warning_group_inspector(snapshot: &Snapshot, code: &str) -> Option<WarningGroupInspector> {
    let mut count = 0;
    let mut samples = Vec::new();
    let mut max_severity: Option<WarningSeverity> = None;
    for warning in &snapshot.warnings {
        if warning.code != code {
            continue;
        }
        count += 1;
        if samples.len() < 10
            && let Some(entity) = &warning.entity
        {
            samples.push(entity.clone());
        }
        max_severity = match (&max_severity, &warning.severity) {
            (None, sev) => Some(sev.clone()),
            (Some(WarningSeverity::Error), _) => max_severity,
            (Some(_), WarningSeverity::Error) => Some(WarningSeverity::Error),
            (Some(WarningSeverity::Warning), WarningSeverity::Info) => max_severity,
            (Some(_), sev) => Some(sev.clone()),
        };
    }
    if count == 0 {
        return None;
    }
    let meta = lazyadmin_core::doctor::classify(code);
    let label = if meta.code == "unknown" {
        code.to_string()
    } else {
        meta.label.to_string()
    };
    Some(WarningGroupInspector {
        code: code.to_string(),
        title: format!("warning {code}"),
        label,
        remediation: meta.remediation.to_string(),
        tier: meta.tier,
        severity: max_severity.unwrap_or(WarningSeverity::Warning),
        count,
        sample_entities: samples,
    })
}

// ─── shared helpers ────────────────────────────────────────────────

fn listener_label(listener: &Listener) -> String {
    match (
        listener.bind_addr.as_deref(),
        listener.port,
        listener.path.as_ref(),
    ) {
        (Some(addr), Some(port), _) => format!("{addr}:{port}"),
        (_, Some(port), _) => format!(":{port}"),
        (_, _, Some(path)) => path.display().to_string(),
        _ => listener.id.to_string(),
    }
}

fn listener_bind(listener: &Listener) -> String {
    if let Some(path) = &listener.path {
        return path.display().to_string();
    }
    let addr = listener.bind_addr.as_deref().unwrap_or("*");
    match listener.port {
        Some(port) => format!("{addr}:{port}"),
        None => addr.to_string(),
    }
}

fn first_owner_pid(listener: &Listener, snapshot: &Snapshot) -> Option<i32> {
    for owner in &listener.owners {
        match owner {
            EntityRef::Process(key) => return Some(key.pid),
            EntityRef::Workload(id) => {
                if let Some(pid) = snapshot
                    .workloads
                    .iter()
                    .find(|workload| &workload.id == id)
                    .and_then(|workload| workload.pids.first())
                    .map(|key| key.pid)
                {
                    return Some(pid);
                }
            }
            _ => {}
        }
    }
    None
}

fn owner_label(listener: &Listener, snapshot: &Snapshot) -> String {
    match listener.owners.first() {
        Some(EntityRef::Process(key)) => format!("pid {}", key.pid),
        Some(EntityRef::Workload(id)) => snapshot
            .workloads
            .iter()
            .find(|workload| &workload.id == id)
            .map(|workload| workload.display_name.clone())
            .unwrap_or_else(|| format!("workload {id}")),
        Some(EntityRef::Manager(id)) => snapshot
            .managers
            .iter()
            .find(|manager| &manager.id == id)
            .map(|manager| manager.name.clone())
            .unwrap_or_else(|| format!("manager {id}")),
        Some(EntityRef::Run(id)) => format!("tracked run {id}"),
        Some(other) => format!("{other:?}"),
        None => "unowned".to_string(),
    }
}

fn listeners_for_pid(snapshot: &Snapshot, pid: i32, exclude: &ListenerId) -> Vec<RelatedListener> {
    let mut out = Vec::new();
    for listener in &snapshot.listeners {
        if &listener.id == exclude {
            continue;
        }
        let owns_pid = listener.owners.iter().any(|owner| match owner {
            EntityRef::Process(key) => key.pid == pid,
            EntityRef::Workload(id) => snapshot
                .workloads
                .iter()
                .find(|workload| &workload.id == id)
                .is_some_and(|workload| workload.pids.iter().any(|key| key.pid == pid)),
            _ => false,
        });
        if owns_pid {
            out.push(RelatedListener {
                listener_id: listener.id.clone(),
                bind: listener_bind(listener),
                exposure: listener.exposure.clone(),
            });
        }
    }
    out
}

fn listener_project(listener: &Listener, snapshot: &Snapshot) -> Option<ProjectRefBlock> {
    for owner in &listener.owners {
        let project_id: Option<&ProjectId> = match owner {
            EntityRef::Workload(id) => snapshot
                .workloads
                .iter()
                .find(|workload| &workload.id == id)
                .and_then(|workload| workload.project.as_ref()),
            EntityRef::Process(key) => snapshot
                .workloads
                .iter()
                .find(|workload| workload.pids.iter().any(|k| k == key))
                .and_then(|workload| workload.project.as_ref()),
            _ => None,
        };
        if let Some(project_id) = project_id
            && let Some(project) = snapshot.projects.iter().find(|p| &p.id == project_id)
        {
            return Some(ProjectRefBlock {
                project_id: project.id.clone(),
                name: project.name.clone(),
                root: project.root.display().to_string(),
            });
        }
    }
    None
}

fn process_fragment(process: &Process, snapshot: &Snapshot) -> ProcessFragment {
    let children: Vec<i32> = snapshot
        .processes
        .iter()
        .filter(|child| child.ppid == Some(process.pid) && child.pid != process.pid)
        .map(|child| child.pid)
        .take(32)
        .collect();
    ProcessFragment {
        pid: process.pid,
        cmdline_full: process.cmdline.join(" "),
        exe: process.exe.as_ref().map(|p| p.display().to_string()),
        cwd: process.cwd.as_ref().map(|p| p.display().to_string()),
        user: process.user.clone(),
        parent_pid: process.ppid,
        children,
        key: process.key.clone(),
    }
}

fn warnings_for(snapshot: &Snapshot, predicate: impl Fn(&Warning) -> bool) -> Vec<WarningRef> {
    snapshot
        .warnings
        .iter()
        .filter(|w| predicate(w))
        .map(|w| WarningRef {
            code: w.code.clone(),
            severity: w.severity.clone(),
            message: w.message.clone(),
        })
        .collect()
}

fn build_confidence(value: Confidence, provenance: &[Provenance]) -> ConfidenceBlock {
    // Aggregate by adapter so we don't repeat the same signal class
    // five times for one entity. Within an adapter, prefer the
    // highest-confidence claim.
    let mut by_adapter: HashMap<String, (Confidence, String)> = HashMap::new();
    for entry in provenance {
        by_adapter
            .entry(entry.adapter.clone())
            .and_modify(|(conf, claim)| {
                if entry.confidence > *conf {
                    *conf = entry.confidence;
                    *claim = entry.claim.clone();
                }
            })
            .or_insert((entry.confidence, entry.claim.clone()));
    }
    let mut signals: Vec<ConfidenceSignalEntry> = by_adapter
        .into_iter()
        .map(|(adapter, (_, claim))| ConfidenceSignalEntry {
            signal: ConfidenceSignal::classify(&adapter),
            adapter,
            claim,
        })
        .collect();
    signals.sort_by(|a, b| a.adapter.cmp(&b.adapter));
    ConfidenceBlock { value, signals }
}

// ── action previews ─────────────────────────────────────────────────

fn listener_actions(listener: &Listener) -> Vec<ActionPreview> {
    let bind = listener_bind(listener);
    let port = listener.port;
    let mut actions = Vec::new();
    if let Some(port) = port {
        actions.push(ActionPreview {
            verb: "free port".into(),
            key_hint: "f".into(),
            command_string: format!("lazyadmin free {port}"),
            enabled: true,
            disabled_reason: None,
        });
    } else {
        actions.push(ActionPreview {
            verb: "free port".into(),
            key_hint: "f".into(),
            command_string: format!("lazyadmin free {bind}"),
            enabled: false,
            disabled_reason: Some("no port (unix socket listener)".into()),
        });
    }
    actions.push(ActionPreview {
        verb: "logs".into(),
        key_hint: "L".into(),
        command_string: format!("lazyadmin logs --listener {}", listener.id),
        enabled: true,
        disabled_reason: None,
    });
    actions
}

fn workload_actions(workload: &Workload) -> Vec<ActionPreview> {
    let mut actions = Vec::new();
    let direct = matches!(workload.runtime, RuntimeKind::Direct);
    actions.push(ActionPreview {
        verb: "restart".into(),
        key_hint: "r".into(),
        command_string: format!("lazyadmin restart {}", workload.id),
        enabled: !direct,
        disabled_reason: direct.then(|| "direct process — no manager to restart it".into()),
    });
    actions.push(ActionPreview {
        verb: "stop".into(),
        key_hint: "s".into(),
        command_string: format!("lazyadmin stop {}", workload.id),
        enabled: !direct,
        disabled_reason: direct.then(|| "direct process — use kill on the pid instead".into()),
    });
    actions.push(ActionPreview {
        verb: "logs".into(),
        key_hint: "L".into(),
        command_string: format!("lazyadmin logs --workload {}", workload.id),
        enabled: !direct,
        disabled_reason: direct.then(|| "logs unavailable for direct processes".into()),
    });
    actions
}

fn process_actions(process: &Process) -> Vec<ActionPreview> {
    vec![
        ActionPreview {
            verb: "kill".into(),
            key_hint: "k".into(),
            command_string: format!("kill {}", process.pid),
            enabled: true,
            disabled_reason: None,
        },
        ActionPreview {
            verb: "kill -9".into(),
            key_hint: "K".into(),
            command_string: format!("kill -9 {}", process.pid),
            enabled: true,
            disabled_reason: None,
        },
        ActionPreview {
            verb: "logs".into(),
            key_hint: "L".into(),
            command_string: format!("lazyadmin logs --pid {}", process.pid),
            enabled: process.lazyadmin_run_id.is_some(),
            disabled_reason: process
                .lazyadmin_run_id
                .is_none()
                .then(|| "logs only available for lazyadmin-tracked runs".into()),
        },
    ]
}

fn tracked_run_actions(run: &TrackedRun) -> Vec<ActionPreview> {
    vec![
        ActionPreview {
            verb: "logs".into(),
            key_hint: "L".into(),
            command_string: format!("lazyadmin logs --run {}", run.id),
            enabled: true,
            disabled_reason: None,
        },
        ActionPreview {
            verb: "forget".into(),
            key_hint: "F".into(),
            command_string: format!("lazyadmin run forget {}", run.id),
            enabled: matches!(run.state, WorkloadState::Stopped | WorkloadState::Exited),
            disabled_reason: (!matches!(run.state, WorkloadState::Stopped | WorkloadState::Exited))
                .then(|| "run is still active — stop it first".into()),
        },
    ]
}

// ─── section flatteners ────────────────────────────────────────────

fn listener_sections(v: &ListenerInspector) -> Vec<InspectorSection> {
    let mut out = Vec::new();
    out.push(InspectorSection {
        heading: "IDENTITY",
        rows: identity_rows(v),
    });
    if let Some(process) = &v.process {
        out.push(InspectorSection {
            heading: "PROCESS",
            rows: process_rows(process),
        });
    }
    if !v.related_listeners.is_empty() {
        out.push(InspectorSection {
            heading: "RELATED",
            rows: related_listener_rows(&v.related_listeners),
        });
    }
    if let Some(project) = &v.project {
        out.push(InspectorSection {
            heading: "PROJECT",
            rows: vec![
                row("name", &project.name).jump(JumpTarget::Project {
                    id: project.project_id.clone(),
                }),
                row("root", &project.root),
            ],
        });
    }
    out.push(InspectorSection {
        heading: "CONFIDENCE",
        rows: confidence_rows(&v.confidence),
    });
    if !v.actions.is_empty() {
        out.push(InspectorSection {
            heading: "ACTIONS",
            rows: action_rows(&v.actions),
        });
    }
    if !v.warnings.is_empty() {
        out.push(InspectorSection {
            heading: "WARNINGS",
            rows: warning_rows(&v.warnings),
        });
    }
    out
}

fn identity_rows(v: &ListenerInspector) -> Vec<InspectorRow> {
    let id = &v.identity;
    let mut rows = vec![
        row("listener id", &id.listener_id),
        row("bind", &id.bind),
        row("protocol", &format!("{:?}", id.protocol)),
        row("family", &format!("{:?}", id.family)),
        row("exposure", &format!("{:?}", id.exposure)),
        row("state", &format!("{:?}", id.state)),
        row("netns", &id.netns),
        row("owner", &id.owner_label),
    ];
    if let Some(user) = &id.user {
        rows.push(row("user", user));
    }
    rows
}

fn process_rows(p: &ProcessFragment) -> Vec<InspectorRow> {
    let mut rows = vec![
        row("pid", &p.pid.to_string()).jump(JumpTarget::Process { key: p.key.clone() }),
        row("command", &p.cmdline_full),
    ];
    if let Some(exe) = &p.exe {
        rows.push(row("exe", exe));
    }
    if let Some(cwd) = &p.cwd {
        rows.push(row("cwd", cwd));
    }
    if let Some(user) = &p.user {
        rows.push(row("user", user));
    }
    if let Some(parent_pid) = p.parent_pid {
        rows.push(row("parent pid", &parent_pid.to_string()));
    }
    if !p.children.is_empty() {
        let count = p.children.len();
        let preview = p
            .children
            .iter()
            .take(5)
            .map(i32::to_string)
            .collect::<Vec<_>>()
            .join(", ");
        let value = if count > 5 {
            format!("{preview}, … (+{} more)", count - 5)
        } else {
            preview
        };
        rows.push(row("children", &value).with_secondary(&format!("{count} pids")));
    }
    rows
}

fn related_listener_rows(items: &[RelatedListener]) -> Vec<InspectorRow> {
    items
        .iter()
        .map(|item| {
            row(
                &format!("{:?}", item.exposure),
                &format!("{} ({})", item.bind, item.listener_id),
            )
            .jump(JumpTarget::Listener {
                id: item.listener_id.clone(),
            })
        })
        .collect()
}

fn confidence_rows(block: &ConfidenceBlock) -> Vec<InspectorRow> {
    let mut rows = vec![row("value", &format!("{:?}", block.value))];
    for entry in &block.signals {
        let label = entry.signal.label();
        rows.push(row(label, &entry.claim).with_secondary(&entry.adapter));
    }
    if block.signals.is_empty() {
        rows.push(
            row("note", "no provenance recorded — confidence is best-effort")
                .with_secondary("BestEffort"),
        );
    }
    rows
}

fn action_rows(actions: &[ActionPreview]) -> Vec<InspectorRow> {
    actions
        .iter()
        .map(|a| {
            let mut value = format!("[{}] {}", a.key_hint, a.verb);
            if !a.enabled
                && let Some(reason) = &a.disabled_reason
            {
                value.push_str(&format!(" — disabled ({reason})"));
            }
            row(&value, &a.command_string)
        })
        .collect()
}

fn warning_rows(warnings: &[WarningRef]) -> Vec<InspectorRow> {
    warnings
        .iter()
        .map(|w| {
            row(
                &format!("{:?}", w.severity),
                &format!("{}: {}", w.code, w.message),
            )
            .jump(JumpTarget::WarningGroup {
                code: w.code.clone(),
            })
        })
        .collect()
}

fn workload_sections(v: &WorkloadInspector) -> Vec<InspectorSection> {
    let mut out = Vec::new();
    let id = &v.identity;
    let mut identity_rows = vec![
        row("workload id", &id.workload_id),
        row("display name", &id.display_name),
        row("runtime", &format!("{:?}", id.runtime)),
        row("state", &format!("{:?}", id.state)),
    ];
    if let Some(health) = &id.health {
        identity_rows.push(row("health", health));
    }
    if let Some(policy) = &id.restart_policy {
        identity_rows.push(row("restart policy", policy));
    }
    out.push(InspectorSection {
        heading: "IDENTITY",
        rows: identity_rows,
    });
    if !v.child_processes.is_empty() {
        let mut rows = Vec::new();
        for p in v.child_processes.iter().take(10) {
            rows.push(
                row(&format!("pid {}", p.pid), &p.cmdline_full)
                    .jump(JumpTarget::Process { key: p.key.clone() }),
            );
        }
        if v.child_processes.len() > 10 {
            rows.push(row(
                "more",
                &format!("+{} children", v.child_processes.len() - 10),
            ));
        }
        out.push(InspectorSection {
            heading: "PROCESS",
            rows,
        });
    }
    if !v.listeners.is_empty() {
        out.push(InspectorSection {
            heading: "RELATED",
            rows: related_listener_rows(&v.listeners),
        });
    }
    if let Some(project) = &v.project {
        out.push(InspectorSection {
            heading: "PROJECT",
            rows: vec![
                row("name", &project.name).jump(JumpTarget::Project {
                    id: project.project_id.clone(),
                }),
                row("root", &project.root),
            ],
        });
    }
    if let Some(manager) = &v.manager {
        out.push(InspectorSection {
            heading: "MANAGER",
            rows: vec![
                row("name", &manager.name).jump(JumpTarget::Manager {
                    id: manager.manager_id.clone(),
                }),
                row("kind", &format!("{:?}", manager.kind)),
            ],
        });
    }
    out.push(InspectorSection {
        heading: "CONFIDENCE",
        rows: confidence_rows(&v.confidence),
    });
    if !v.actions.is_empty() {
        out.push(InspectorSection {
            heading: "ACTIONS",
            rows: action_rows(&v.actions),
        });
    }
    if !v.warnings.is_empty() {
        out.push(InspectorSection {
            heading: "WARNINGS",
            rows: warning_rows(&v.warnings),
        });
    }
    out
}

fn process_sections(v: &ProcessInspector) -> Vec<InspectorSection> {
    let mut out = Vec::new();
    out.push(InspectorSection {
        heading: "IDENTITY",
        rows: process_rows(&v.identity),
    });
    if !v.listeners.is_empty() {
        out.push(InspectorSection {
            heading: "RELATED",
            rows: related_listener_rows(&v.listeners),
        });
    }
    if let Some(workload) = &v.workload {
        out.push(InspectorSection {
            heading: "WORKLOAD",
            rows: vec![
                row("name", &workload.display_name).jump(JumpTarget::Workload {
                    id: workload.workload_id.clone(),
                }),
                row("runtime", &format!("{:?}", workload.runtime)),
            ],
        });
    }
    if let Some(run) = &v.tracked_run {
        out.push(InspectorSection {
            heading: "TRACKED RUN",
            rows: vec![
                row("run id", &run.run_id.to_string()).jump(JumpTarget::TrackedRun {
                    id: run.run_id.clone(),
                }),
                row("tag", run.tag.as_deref().unwrap_or("")),
            ]
            .into_iter()
            .filter(|r| !r.value.is_empty())
            .collect(),
        });
    }
    out.push(InspectorSection {
        heading: "CONFIDENCE",
        rows: confidence_rows(&v.confidence),
    });
    if !v.actions.is_empty() {
        out.push(InspectorSection {
            heading: "ACTIONS",
            rows: action_rows(&v.actions),
        });
    }
    if !v.warnings.is_empty() {
        out.push(InspectorSection {
            heading: "WARNINGS",
            rows: warning_rows(&v.warnings),
        });
    }
    out
}

fn project_sections(v: &ProjectInspector) -> Vec<InspectorSection> {
    let mut out = Vec::new();
    let id = &v.identity;
    let mut identity_rows = vec![
        row("project id", &id.project_id),
        row("name", &id.name),
        row("root", &id.root),
    ];
    if let Some(remote) = &id.git_remote {
        identity_rows.push(row("git remote", remote));
    }
    if let Some(pm) = &id.package_manager {
        identity_rows.push(row("package manager", pm));
    }
    out.push(InspectorSection {
        heading: "IDENTITY",
        rows: identity_rows,
    });
    if !v.workloads.is_empty() {
        let rows = v
            .workloads
            .iter()
            .map(|w| {
                row(&format!("{:?}", w.runtime), &w.display_name).jump(JumpTarget::Workload {
                    id: w.workload_id.clone(),
                })
            })
            .collect();
        out.push(InspectorSection {
            heading: "WORKLOADS",
            rows,
        });
    }
    if !v.listeners.is_empty() {
        out.push(InspectorSection {
            heading: "RELATED",
            rows: related_listener_rows(&v.listeners),
        });
    }
    if !v.markers.is_empty() {
        out.push(InspectorSection {
            heading: "MARKERS",
            rows: v
                .markers
                .iter()
                .map(|m| row("marker", m))
                .collect::<Vec<_>>(),
        });
    }
    out
}

fn manager_sections(v: &ManagerInspector) -> Vec<InspectorSection> {
    let mut out = Vec::new();
    let id = &v.identity;
    let mut identity_rows = vec![
        row("manager id", &id.manager_id),
        row("name", &id.name),
        row("kind", &format!("{:?}", id.kind)),
        row("scope", &format!("{:?}", id.scope)),
        row("available", &id.available.to_string()),
        row("permission", &format!("{:?}", id.permission)),
    ];
    if let Some(version) = &id.version {
        identity_rows.push(row("version", version));
    }
    if let Some(socket) = &id.socket {
        identity_rows.push(row("socket", socket));
    }
    out.push(InspectorSection {
        heading: "IDENTITY",
        rows: identity_rows,
    });
    if !v.managed_workloads.is_empty() {
        out.push(InspectorSection {
            heading: "MANAGED WORKLOADS",
            rows: v
                .managed_workloads
                .iter()
                .map(|w| {
                    row(&format!("{:?}", w.runtime), &w.display_name).jump(JumpTarget::Workload {
                        id: w.workload_id.clone(),
                    })
                })
                .collect(),
        });
    }
    out
}

fn tracked_run_sections(v: &TrackedRunInspector) -> Vec<InspectorSection> {
    let mut out = Vec::new();
    let id = &v.identity;
    let mut identity_rows = vec![row("run id", &id.run_id)];
    if let Some(tag) = &id.tag {
        identity_rows.push(row("tag", tag));
    }
    identity_rows.push(row("command", &id.command));
    if let Some(cwd) = &id.cwd {
        identity_rows.push(row("cwd", cwd));
    }
    identity_rows.push(row("state", &format!("{:?}", id.state)));
    out.push(InspectorSection {
        heading: "IDENTITY",
        rows: identity_rows,
    });
    if let Some(workload) = &v.workload {
        out.push(InspectorSection {
            heading: "WORKLOAD",
            rows: vec![
                row("name", &workload.display_name).jump(JumpTarget::Workload {
                    id: workload.workload_id.clone(),
                }),
            ],
        });
    }
    if !v.actions.is_empty() {
        out.push(InspectorSection {
            heading: "ACTIONS",
            rows: action_rows(&v.actions),
        });
    }
    out
}

fn warning_group_sections(v: &WarningGroupInspector) -> Vec<InspectorSection> {
    let mut rows = vec![
        row("code", &v.code),
        row("label", &v.label),
        row("severity", &format!("{:?}", v.severity)),
        row("tier", &format!("{:?}", v.tier)),
        row("count", &v.count.to_string()),
        row("remediation", &v.remediation),
    ];
    if !v.sample_entities.is_empty() {
        rows.push(row(
            "samples",
            &v.sample_entities
                .iter()
                .map(|e| format!("{e:?}"))
                .collect::<Vec<_>>()
                .join("\n"),
        ));
    }
    vec![InspectorSection {
        heading: "WARNING GROUP",
        rows,
    }]
}

// ─── row helpers ───────────────────────────────────────────────────

fn row(label: &str, value: &str) -> InspectorRow {
    InspectorRow {
        label: label.to_string(),
        value: value.to_string(),
        secondary: None,
        jump_target: None,
    }
}

impl InspectorRow {
    fn with_secondary(mut self, secondary: &str) -> Self {
        self.secondary = Some(secondary.to_string());
        self
    }

    fn jump(mut self, target: JumpTarget) -> Self {
        self.jump_target = Some(target);
        self
    }
}

// ─── tests ─────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn now() -> chrono::DateTime<chrono::Utc> {
        chrono::Utc.with_ymd_and_hms(2026, 4, 30, 12, 0, 0).unwrap()
    }

    fn process_key(pid: i32) -> ProcessKey {
        ProcessKey {
            pid,
            boot_id: "boot".into(),
            start_time_ticks: pid as u64,
        }
    }

    fn listener(
        id: impl Into<String>,
        addr: &str,
        port: u16,
        exposure: Exposure,
        owners: Vec<EntityRef>,
    ) -> Listener {
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
            provenance: vec![Provenance {
                adapter: "procfs".into(),
                claim: "matched fd → socket inode".into(),
                evidence: "/proc/17385/fd/9".into(),
                confidence: Confidence::High,
                timestamp: now(),
            }],
            first_seen: now(),
            last_seen: now(),
            dual_stack_state: DualStackState::Unknown,
        }
    }

    fn process(pid: i32, cmdline: &[&str]) -> Process {
        Process {
            key: process_key(pid),
            pid,
            start_time_ticks: pid as u64,
            boot_id: "boot".into(),
            user: Some("dev".into()),
            exe: Some(format!("/usr/bin/proc-{pid}").into()),
            cmdline: cmdline.iter().map(|&s| s.into()).collect(),
            cwd: Some("/home/dev/work".into()),
            ppid: Some(1),
            pgid: None,
            sid: None,
            cgroup: None,
            netns: None,
            container_id: None,
            systemd_unit: None,
            lazyadmin_run_id: None,
            environment: RedactedEnvironmentSummary::default(),
            provenance: vec![Provenance {
                adapter: "procfs".into(),
                claim: "scanned /proc".into(),
                evidence: "/proc/17385".into(),
                confidence: Confidence::High,
                timestamp: now(),
            }],
        }
    }

    #[test]
    fn inspector_listener_lists_related_listeners_owned_by_same_pid() {
        let mut snapshot = Snapshot::empty();
        let key = process_key(17385);
        snapshot
            .processes
            .push(process(17385, &["node", "server.js"]));
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
            vec![EntityRef::Process(key.clone())],
        ));
        snapshot.listeners.push(listener(
            "tcp:127.0.0.1:9229",
            "127.0.0.1",
            9229,
            Exposure::Loopback,
            vec![EntityRef::Process(key)],
        ));

        let view = InspectorView::lookup(&snapshot, "listener", "tcp:0.0.0.0:3000").expect("found");
        let InspectorView::Listener(listener) = view else {
            panic!("expected listener variant");
        };
        let related_ids: Vec<_> = listener
            .related_listeners
            .iter()
            .map(|r| r.listener_id.to_string())
            .collect();
        assert!(related_ids.contains(&"tcp:0.0.0.0:3001".to_string()));
        assert!(related_ids.contains(&"tcp:127.0.0.1:9229".to_string()));
        assert!(!related_ids.contains(&"tcp:0.0.0.0:3000".to_string()));
    }

    #[test]
    fn inspector_process_lists_listeners_held_by_pid() {
        let mut snapshot = Snapshot::empty();
        let key = process_key(42);
        snapshot
            .processes
            .push(process(42, &["python", "-m", "http.server"]));
        snapshot.listeners.push(listener(
            "tcp:0.0.0.0:8000",
            "0.0.0.0",
            8000,
            Exposure::Public,
            vec![EntityRef::Process(key.clone())],
        ));
        let view = InspectorView::lookup(&snapshot, "process", "42").expect("found process");
        let InspectorView::Process(process_view) = view else {
            panic!("expected process variant");
        };
        assert_eq!(process_view.identity.pid, 42);
        assert_eq!(process_view.listeners.len(), 1);
        assert_eq!(process_view.listeners[0].bind, "0.0.0.0:8000");
    }

    #[test]
    fn inspector_listener_id_is_not_truncated_in_view_model() {
        let mut snapshot = Snapshot::empty();
        let long_id = "tcp:[fd7a:115c:a1e0:abcd:efef:1234:5678:9abc]:65535";
        snapshot.listeners.push(listener(
            long_id,
            "fd7a:115c:a1e0:abcd:efef:1234:5678:9abc",
            65535,
            Exposure::Public,
            vec![],
        ));
        let view = InspectorView::lookup(&snapshot, "listener", long_id).expect("found");
        let InspectorView::Listener(listener) = view else {
            panic!("expected listener variant");
        };
        assert_eq!(listener.identity.listener_id, long_id);
        // No section row truncates the identifier.
        for section in listener.sections() {
            for r in &section.rows {
                if r.label == "listener id" {
                    assert_eq!(r.value, long_id);
                }
            }
        }
    }

    #[test]
    fn inspector_no_dash_rows_in_any_variant() {
        let mut snapshot = Snapshot::empty();
        // Bare listener with no owner and no port to maximize the
        // chance of accidental "-" rows.
        snapshot.listeners.push(Listener {
            id: ListenerId::new("unix:/tmp/sock"),
            protocol: Protocol::Tcp,
            family: AddressFamily::Unix,
            bind_addr: None,
            port: None,
            path: Some("/tmp/sock".into()),
            state: ListenerState::Listen,
            netns: "host".into(),
            socket_inode: None,
            exposure: Exposure::UnixLocal,
            owners: vec![],
            confidence: Confidence::Low,
            provenance: vec![],
            first_seen: now(),
            last_seen: now(),
            dual_stack_state: DualStackState::Unknown,
        });
        snapshot.processes.push(process(1, &["init"]));
        snapshot.workloads.push(Workload {
            id: WorkloadId::new("workload:lone"),
            display_name: "lone".into(),
            runtime: RuntimeKind::Direct,
            state: WorkloadState::Running,
            pids: vec![process_key(1)],
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
        snapshot.projects.push(Project {
            id: ProjectId::new("project:lone"),
            root: "/tmp/lone".into(),
            name: "lone-project".into(),
            markers: vec![],
            git_remote: None,
            package_manager: None,
            dev_commands: vec![],
            provenance: vec![],
        });
        snapshot.managers.push(Manager {
            id: ManagerId::new("manager:test"),
            kind: RuntimeKind::Direct,
            name: "tester".into(),
            scope: ManagerScope::User,
            socket: None,
            available: true,
            permission: PermissionState::Ok,
            version: None,
            provenance: vec![],
        });
        snapshot.tracked_runs.push(TrackedRun {
            id: RunId::new("run:tester"),
            tag: None,
            command: vec!["sleep".into(), "1".into()],
            cwd: None,
            state: WorkloadState::Stopped,
            started_at: Some(now()),
            provenance: vec![],
        });
        snapshot.warnings.push(Warning {
            severity: WarningSeverity::Warning,
            code: "PUBLIC".into(),
            message: "unowned listener".into(),
            entity: Some(EntityRef::Listener(ListenerId::new("unix:/tmp/sock"))),
            provenance: vec![],
        });

        for view in InspectorView::all_from_snapshot(&snapshot) {
            for section in view.to_sections() {
                for row in &section.rows {
                    assert_ne!(
                        row.value.trim(),
                        "-",
                        "section {} row {}",
                        section.heading,
                        row.label
                    );
                    assert_ne!(
                        row.value.trim(),
                        "unavailable",
                        "section {} row {}",
                        section.heading,
                        row.label
                    );
                }
            }
        }
        // Plus the warning-group inspector.
        let view = InspectorView::lookup(&snapshot, "warning_group", "PUBLIC").expect("present");
        for section in view.to_sections() {
            for row in &section.rows {
                assert_ne!(row.value.trim(), "-");
            }
        }
    }

    #[test]
    fn inspector_confidence_explains_which_signal_is_best_effort() {
        let mut snapshot = Snapshot::empty();
        let key = process_key(99);
        snapshot.processes.push(process(99, &["server"]));
        let mut listener_rec = listener(
            "tcp:0.0.0.0:7000",
            "0.0.0.0",
            7000,
            Exposure::Public,
            vec![EntityRef::Process(key)],
        );
        listener_rec.provenance.push(Provenance {
            adapter: "container-bollard".into(),
            claim: "matched container inspect".into(),
            evidence: "container_id=abc".into(),
            confidence: Confidence::Medium,
            timestamp: now(),
        });
        listener_rec.provenance.push(Provenance {
            adapter: "best-effort-fallback".into(),
            claim: "no precise signal".into(),
            evidence: "".into(),
            confidence: Confidence::Low,
            timestamp: now(),
        });
        snapshot.listeners.push(listener_rec);
        let view = InspectorView::lookup(&snapshot, "listener", "tcp:0.0.0.0:7000").expect("found");
        let InspectorView::Listener(listener) = view else {
            panic!("expected listener variant");
        };
        let labels: Vec<_> = listener
            .confidence
            .signals
            .iter()
            .map(|s| s.signal.label())
            .collect();
        assert!(labels.contains(&"procfs PID→socket inode"));
        assert!(labels.contains(&"container runtime inspect"));
        assert!(labels.contains(&"best-effort fallback"));
    }

    #[test]
    fn restart_disabled_for_direct_process_with_explicit_reason() {
        let mut snapshot = Snapshot::empty();
        snapshot.workloads.push(Workload {
            id: WorkloadId::new("workload:direct"),
            display_name: "direct-thing".into(),
            runtime: RuntimeKind::Direct,
            state: WorkloadState::Running,
            pids: vec![],
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
        let view = InspectorView::lookup(&snapshot, "workload", "workload:direct").expect("found");
        let InspectorView::Workload(workload) = view else {
            panic!("expected workload variant");
        };
        let restart = workload
            .actions
            .iter()
            .find(|a| a.verb == "restart")
            .expect("restart action present");
        assert!(!restart.enabled);
        let reason = restart
            .disabled_reason
            .as_ref()
            .expect("disabled reason set");
        assert!(
            reason.to_lowercase().contains("direct"),
            "expected reason to mention direct process, got {reason}"
        );
    }

    #[test]
    fn command_preview_string_matches_expected_form() {
        let mut snapshot = Snapshot::empty();
        snapshot.listeners.push(listener(
            "tcp:0.0.0.0:5000",
            "0.0.0.0",
            5000,
            Exposure::Public,
            vec![],
        ));
        let view = InspectorView::lookup(&snapshot, "listener", "tcp:0.0.0.0:5000").expect("found");
        let InspectorView::Listener(listener) = view else {
            panic!("expected listener variant");
        };
        let free = listener
            .actions
            .iter()
            .find(|a| a.verb == "free port")
            .expect("free action");
        assert_eq!(free.command_string, "lazyadmin free 5000");
    }
}

#[cfg(test)]
impl ListenerInspector {
    fn sections(&self) -> Vec<InspectorSection> {
        listener_sections(self)
    }
}
