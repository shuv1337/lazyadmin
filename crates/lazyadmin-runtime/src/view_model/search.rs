use std::{path::PathBuf, time::Instant};

use fuzzy_matcher::{FuzzyMatcher, skim::SkimMatcherV2};
use lazyadmin_core::model::{
    EntityRef, Exposure, Listener, ListenerId, Manager, ManagerId, ManagerScope, Process,
    ProcessKey, Project, ProjectId, Protocol, Snapshot, Workload, WorkloadId,
};
use serde::{Deserialize, Serialize};
use tracing::info;

use super::RAIL_ENTRIES;

pub const SEARCH_SCHEMA_VERSION: &str = "lazyadmin.search.v1";
pub const DEFAULT_SEARCH_LIMIT: usize = 200;
pub const MAX_SEARCH_LIMIT: usize = 500;

/// Filters which entity kinds the runtime matcher considers. All `true` by
/// default. Callers that want a narrower scope (CLI, palette, future Web
/// endpoints) can flip individual flags off without changing the surrounding
/// pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SearchKinds {
    pub listeners: bool,
    pub processes: bool,
    pub workloads: bool,
    pub projects: bool,
    pub managers: bool,
    pub rail_views: bool,
}

impl Default for SearchKinds {
    fn default() -> Self {
        Self {
            listeners: true,
            processes: true,
            workloads: true,
            projects: true,
            managers: true,
            rail_views: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct SearchOptions {
    pub limit: usize,
    pub show_system: bool,
    pub kinds: SearchKinds,
}

impl Default for SearchOptions {
    fn default() -> Self {
        Self {
            limit: DEFAULT_SEARCH_LIMIT,
            show_system: true,
            kinds: SearchKinds::default(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchQuery {
    pub raw: String,
    pub normalized: String,
    pub kind: SearchKind,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq, Default)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SearchKind {
    #[default]
    Empty,
    Text {
        text: String,
    },
    Port {
        port: u16,
    },
    Pid {
        pid: i32,
    },
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchResults {
    pub schema_version: String,
    pub query: SearchQuery,
    pub listeners: SearchGroup<ListenerHit>,
    pub processes: SearchGroup<ProcessHit>,
    #[serde(default)]
    pub workloads: SearchGroup<WorkloadHit>,
    #[serde(default)]
    pub projects: SearchGroup<ProjectHit>,
    #[serde(default)]
    pub managers: SearchGroup<ManagerHit>,
    #[serde(default)]
    pub rail_views: SearchGroup<RailViewHit>,
    pub strategy_hint: String,
    pub fell_back_to_prefix: bool,
    pub elapsed_ms: u128,
}

impl SearchResults {
    pub fn search_hit_count(&self) -> usize {
        search_hit_count(self)
    }

    pub fn hit_at(&self, index: usize) -> Option<SearchHitRef<'_>> {
        search_hit_at(self, index)
    }
}

impl Default for SearchResults {
    fn default() -> Self {
        Self {
            schema_version: SEARCH_SCHEMA_VERSION.to_string(),
            query: SearchQuery {
                raw: String::new(),
                normalized: String::new(),
                kind: SearchKind::Empty,
            },
            listeners: SearchGroup::default(),
            processes: SearchGroup::default(),
            workloads: SearchGroup::default(),
            projects: SearchGroup::default(),
            managers: SearchGroup::default(),
            rail_views: SearchGroup::default(),
            strategy_hint: String::new(),
            fell_back_to_prefix: false,
            elapsed_ms: 0,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SearchGroup<T> {
    pub total: usize,
    pub returned: usize,
    pub truncated: bool,
    pub hits: Vec<T>,
}

impl<T> Default for SearchGroup<T> {
    fn default() -> Self {
        Self {
            total: 0,
            returned: 0,
            truncated: false,
            hits: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ListenerHit {
    pub id: ListenerId,
    pub port: Option<u16>,
    pub bind: String,
    pub protocol: Protocol,
    pub exposure: Exposure,
    pub owner_label: String,
    pub workload_labels: Vec<String>,
    pub project_label: Option<String>,
    pub score: i64,
    pub matched_indices: Vec<usize>,
    pub is_system: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProcessHit {
    pub key: ProcessKey,
    pub pid: i32,
    pub user: Option<String>,
    pub exe_or_argv0: String,
    pub cmdline_compact: String,
    pub cwd: Option<PathBuf>,
    pub score: i64,
    pub matched_indices: Vec<usize>,
    pub is_system: bool,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct WorkloadHit {
    pub id: WorkloadId,
    pub display_name: String,
    pub runtime: String,
    pub project_label: Option<String>,
    pub manager_label: Option<String>,
    pub listener_count: usize,
    pub pid_count: usize,
    pub score: i64,
    pub matched_indices: Vec<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ProjectHit {
    pub id: ProjectId,
    pub name: String,
    pub root: PathBuf,
    pub package_manager: Option<String>,
    pub git_remote: Option<String>,
    pub score: i64,
    pub matched_indices: Vec<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct ManagerHit {
    pub id: ManagerId,
    pub name: String,
    pub kind: String,
    pub scope: String,
    pub available: bool,
    pub score: i64,
    pub matched_indices: Vec<usize>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct RailViewHit {
    pub id: String,
    pub label: String,
    pub score: i64,
    pub matched_indices: Vec<usize>,
}

fn classify(raw: &str) -> SearchKind {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return SearchKind::Empty;
    }
    if trimmed.chars().all(|c| c.is_ascii_digit()) {
        if let Ok(port) = trimmed.parse::<u16>() {
            return SearchKind::Port { port };
        }
        if let Ok(pid) = trimmed.parse::<i32>() {
            if pid > 0 {
                return SearchKind::Pid { pid };
            }
        }
    }
    SearchKind::Text {
        text: trimmed.into(),
    }
}

fn listener_bind_str(listener: &Listener) -> String {
    match (listener.bind_addr.as_ref(), listener.port) {
        (Some(addr), Some(port)) => format!("{}:{}", addr, port),
        (Some(addr), None) => addr.clone(),
        (None, Some(port)) => format!(":{}", port),
        (None, None) => listener
            .path
            .as_ref()
            .map(|p| p.display().to_string())
            .unwrap_or_default(),
    }
}

fn listener_search_text(listener: &Listener, snapshot: &Snapshot) -> String {
    let mut parts = Vec::new();
    if let Some(port) = listener.port {
        parts.push(port.to_string());
    }
    if let Some(ref addr) = listener.bind_addr {
        parts.push(addr.clone());
    }
    if let Some(ref path) = listener.path {
        parts.push(path.display().to_string());
    }
    parts.push(format!("{:?}", listener.protocol).to_lowercase());
    parts.push(format!("{:?}", listener.exposure).to_lowercase());

    for owner in &listener.owners {
        let label = match owner {
            EntityRef::Process(pk) => snapshot
                .processes
                .iter()
                .find(|p| &p.key == pk)
                .and_then(|p| p.exe.as_ref().map(|e| e.display().to_string()))
                .unwrap_or_else(|| pk.pid.to_string()),
            EntityRef::Workload(wid) => snapshot
                .workloads
                .iter()
                .find(|w| &w.id == wid)
                .map(|w| w.display_name.clone())
                .unwrap_or_else(|| wid.to_string()),
            EntityRef::Manager(mid) => snapshot
                .managers
                .iter()
                .find(|m| &m.id == mid)
                .map(|m| m.name.clone())
                .unwrap_or_else(|| mid.to_string()),
            EntityRef::Project(pid) => snapshot
                .projects
                .iter()
                .find(|p| &p.id == pid)
                .map(|p| p.name.clone())
                .unwrap_or_else(|| pid.to_string()),
            _ => continue,
        };
        parts.push(label);
    }

    for workload in &snapshot.workloads {
        if workload.listeners.iter().any(|lid| lid == &listener.id) {
            parts.push(workload.display_name.clone());
            if let Some(ref pid) = workload.project {
                if let Some(project) = snapshot.projects.iter().find(|p| &p.id == pid) {
                    parts.push(project.name.clone());
                }
            }
        }
    }

    parts.join(" ")
}

fn process_search_text(process: &Process) -> String {
    let mut parts = Vec::new();
    parts.push(process.pid.to_string());
    if let Some(ref user) = process.user {
        parts.push(user.clone());
    }
    if let Some(ref exe) = process.exe {
        parts.push(exe.display().to_string());
    }
    for arg in &process.cmdline {
        parts.push(arg.clone());
    }
    if let Some(ref cwd) = process.cwd {
        parts.push(cwd.display().to_string());
    }
    parts.join(" ")
}

fn workload_search_text(workload: &Workload, snapshot: &Snapshot) -> String {
    let mut parts = Vec::new();
    parts.push(workload.id.to_string());
    parts.push(workload.display_name.clone());
    parts.push(format!("{:?}", workload.runtime).to_lowercase());
    parts.push(format!("{:?}", workload.state).to_lowercase());
    if let Some(ref pid) = workload.project {
        if let Some(project) = snapshot.projects.iter().find(|p| &p.id == pid) {
            parts.push(project.name.clone());
            parts.push(project.root.display().to_string());
        }
    }
    if let Some(ref mid) = workload.manager {
        if let Some(manager) = snapshot.managers.iter().find(|m| &m.id == mid) {
            parts.push(manager.name.clone());
        }
    }
    parts.join(" ")
}

fn project_search_text(project: &Project) -> String {
    let mut parts = Vec::new();
    parts.push(project.name.clone());
    parts.push(project.root.display().to_string());
    if let Some(ref remote) = project.git_remote {
        parts.push(remote.clone());
    }
    if let Some(ref pm) = project.package_manager {
        parts.push(pm.clone());
    }
    for marker in &project.markers {
        parts.push(marker.kind.clone());
    }
    for cmd in &project.dev_commands {
        parts.push(cmd.name.clone());
        parts.push(cmd.command.clone());
    }
    parts.join(" ")
}

fn manager_search_text(manager: &Manager) -> String {
    let mut parts = Vec::new();
    parts.push(manager.name.clone());
    parts.push(format!("{:?}", manager.kind).to_lowercase());
    parts.push(format!("{:?}", manager.scope).to_lowercase());
    if let Some(ref socket) = manager.socket {
        parts.push(socket.display().to_string());
    }
    if let Some(ref v) = manager.version {
        parts.push(v.clone());
    }
    parts.join(" ")
}

fn is_system_listener(listener: &Listener, snapshot: &Snapshot) -> bool {
    for owner in &listener.owners {
        match owner {
            EntityRef::Manager(mid) => {
                if let Some(m) = snapshot.managers.iter().find(|m| &m.id == mid) {
                    if m.scope == ManagerScope::System {
                        return true;
                    }
                }
            }
            EntityRef::Process(pk) => {
                if let Some(p) = snapshot.processes.iter().find(|p| &p.key == pk) {
                    if p.user.as_deref() == Some("root")
                        && p.systemd_unit
                            .as_deref()
                            .map(|u| !u.contains("user"))
                            .unwrap_or(true)
                    {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
}

fn is_system_process(process: &Process) -> bool {
    if process.user.as_deref() == Some("root") {
        if let Some(ref unit) = process.systemd_unit {
            if unit.contains("user") {
                return false;
            }
        }
        return true;
    }
    false
}

fn is_system_workload(workload: &Workload, snapshot: &Snapshot) -> bool {
    if let Some(ref mid) = workload.manager {
        if let Some(m) = snapshot.managers.iter().find(|m| &m.id == mid) {
            if m.scope == ManagerScope::System {
                return true;
            }
        }
    }
    false
}

fn is_system_manager(manager: &Manager) -> bool {
    manager.scope == ManagerScope::System
}

fn entity_ref_label(entity: &EntityRef, snapshot: &Snapshot) -> String {
    match entity {
        EntityRef::Process(pk) => snapshot
            .processes
            .iter()
            .find(|p| &p.key == pk)
            .and_then(|p| p.exe.as_ref().map(|e| e.display().to_string()))
            .unwrap_or_else(|| pk.pid.to_string()),
        EntityRef::Workload(wid) => snapshot
            .workloads
            .iter()
            .find(|w| &w.id == wid)
            .map(|w| w.display_name.clone())
            .unwrap_or_else(|| wid.to_string()),
        EntityRef::Manager(mid) => snapshot
            .managers
            .iter()
            .find(|m| &m.id == mid)
            .map(|m| m.name.clone())
            .unwrap_or_else(|| mid.to_string()),
        EntityRef::Project(pid) => snapshot
            .projects
            .iter()
            .find(|p| &p.id == pid)
            .map(|p| p.name.clone())
            .unwrap_or_else(|| pid.to_string()),
        EntityRef::Listener(lid) => lid.to_string(),
        EntityRef::Run(rid) => rid.to_string(),
        EntityRef::Action(aid) => aid.to_string(),
    }
}

fn workload_labels_for_listener(listener: &Listener, snapshot: &Snapshot) -> Vec<String> {
    let mut labels = Vec::new();
    for workload in &snapshot.workloads {
        if workload.listeners.iter().any(|lid| lid == &listener.id) {
            labels.push(workload.display_name.clone());
        }
    }
    labels
}

fn project_label_for_listener(listener: &Listener, snapshot: &Snapshot) -> Option<String> {
    for workload in &snapshot.workloads {
        if workload.listeners.iter().any(|lid| lid == &listener.id) {
            if let Some(ref pid) = workload.project {
                if let Some(project) = snapshot.projects.iter().find(|p| &p.id == pid) {
                    return Some(project.name.clone());
                }
            }
        }
    }
    None
}

fn project_label_for_workload(workload: &Workload, snapshot: &Snapshot) -> Option<String> {
    workload.project.as_ref().and_then(|pid| {
        snapshot
            .projects
            .iter()
            .find(|p| &p.id == pid)
            .map(|p| p.name.clone())
    })
}

fn manager_label_for_workload(workload: &Workload, snapshot: &Snapshot) -> Option<String> {
    workload.manager.as_ref().and_then(|mid| {
        snapshot
            .managers
            .iter()
            .find(|m| &m.id == mid)
            .map(|m| m.name.clone())
    })
}

fn build_listener_hit(
    listener: &Listener,
    snapshot: &Snapshot,
    score: i64,
    matched_indices: Vec<usize>,
    is_sys: bool,
) -> ListenerHit {
    ListenerHit {
        id: listener.id.clone(),
        port: listener.port,
        bind: listener_bind_str(listener),
        protocol: listener.protocol.clone(),
        exposure: listener.exposure.clone(),
        owner_label: listener
            .owners
            .first()
            .map(|o| entity_ref_label(o, snapshot))
            .unwrap_or_default(),
        workload_labels: workload_labels_for_listener(listener, snapshot),
        project_label: project_label_for_listener(listener, snapshot),
        score,
        matched_indices,
        is_system: is_sys,
    }
}

fn build_process_hit(process: &Process, score: i64, matched_indices: Vec<usize>) -> ProcessHit {
    let exe_or_argv0 = process
        .exe
        .as_ref()
        .map(|e| e.display().to_string())
        .or_else(|| process.cmdline.first().cloned())
        .unwrap_or_default();
    let cmdline_compact = if process.cmdline.is_empty() {
        exe_or_argv0.clone()
    } else {
        process.cmdline.join(" ")
    };
    ProcessHit {
        key: process.key.clone(),
        pid: process.pid,
        user: process.user.clone(),
        exe_or_argv0,
        cmdline_compact,
        cwd: process.cwd.clone(),
        score,
        matched_indices,
        is_system: is_system_process(process),
    }
}

fn build_workload_hit(
    workload: &Workload,
    snapshot: &Snapshot,
    score: i64,
    matched_indices: Vec<usize>,
) -> WorkloadHit {
    WorkloadHit {
        id: workload.id.clone(),
        display_name: workload.display_name.clone(),
        runtime: format!("{:?}", workload.runtime).to_lowercase(),
        project_label: project_label_for_workload(workload, snapshot),
        manager_label: manager_label_for_workload(workload, snapshot),
        listener_count: workload.listeners.len(),
        pid_count: workload.pids.len(),
        score,
        matched_indices,
    }
}

fn build_project_hit(project: &Project, score: i64, matched_indices: Vec<usize>) -> ProjectHit {
    ProjectHit {
        id: project.id.clone(),
        name: project.name.clone(),
        root: project.root.clone(),
        package_manager: project.package_manager.clone(),
        git_remote: project.git_remote.clone(),
        score,
        matched_indices,
    }
}

fn build_manager_hit(manager: &Manager, score: i64, matched_indices: Vec<usize>) -> ManagerHit {
    ManagerHit {
        id: manager.id.clone(),
        name: manager.name.clone(),
        kind: format!("{:?}", manager.kind).to_lowercase(),
        scope: format!("{:?}", manager.scope).to_lowercase(),
        available: manager.available,
        score,
        matched_indices,
    }
}

fn rank_listeners(hits: &mut [ListenerHit]) {
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.port.unwrap_or(u16::MAX).cmp(&b.port.unwrap_or(u16::MAX)))
            .then_with(|| a.bind.cmp(&b.bind))
    });
}

fn rank_processes(hits: &mut [ProcessHit]) {
    hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.pid.cmp(&b.pid)));
}

fn rank_workloads(hits: &mut [WorkloadHit]) {
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then_with(|| a.display_name.cmp(&b.display_name))
    });
}

fn rank_projects(hits: &mut [ProjectHit]) {
    hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.name.cmp(&b.name)));
}

fn rank_managers(hits: &mut [ManagerHit]) {
    hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.name.cmp(&b.name)));
}

fn rank_rail_views(hits: &mut [RailViewHit]) {
    hits.sort_by(|a, b| b.score.cmp(&a.score).then_with(|| a.label.cmp(&b.label)));
}

fn finalize_group<T>(mut hits: Vec<T>, limit: usize) -> SearchGroup<T> {
    let total = hits.len();
    let truncated = total > limit;
    hits.truncate(limit);
    SearchGroup {
        total,
        returned: hits.len(),
        truncated,
        hits,
    }
}

pub fn run(snapshot: &Snapshot, query: &str, options: SearchOptions) -> SearchResults {
    let start = Instant::now();
    let normalized = query.trim().to_string();
    let kind = classify(&normalized);
    let limit = options.limit.clamp(1, MAX_SEARCH_LIMIT);

    let mut results = SearchResults {
        schema_version: SEARCH_SCHEMA_VERSION.to_string(),
        query: SearchQuery {
            raw: query.to_string(),
            normalized: normalized.clone(),
            kind: kind.clone(),
        },
        ..Default::default()
    };

    match kind {
        SearchKind::Empty => {
            results.strategy_hint = String::new();
            results.elapsed_ms = start.elapsed().as_millis();
            return results;
        }
        SearchKind::Port { port } => {
            results.strategy_hint = format!("port :{}", port);
            let mut listener_hits: Vec<ListenerHit> = Vec::new();
            let mut exact_hits = 0usize;

            if options.kinds.listeners {
                for listener in &snapshot.listeners {
                    let is_sys = is_system_listener(listener, snapshot);
                    if is_sys && !options.show_system {
                        continue;
                    }
                    if listener.port == Some(port) {
                        exact_hits += 1;
                        let matched_indices = (0..normalized.len()).collect();
                        listener_hits.push(build_listener_hit(
                            listener,
                            snapshot,
                            10_000,
                            matched_indices,
                            is_sys,
                        ));
                    }
                }
                if exact_hits == 0 {
                    let prefix = normalized.clone();
                    for listener in &snapshot.listeners {
                        if listener.protocol != Protocol::Tcp && listener.protocol != Protocol::Udp
                        {
                            continue;
                        }
                        let is_sys = is_system_listener(listener, snapshot);
                        if is_sys && !options.show_system {
                            continue;
                        }
                        if let Some(p) = listener.port {
                            let port_str = p.to_string();
                            if port_str.starts_with(&prefix) {
                                let matched_indices = (0..prefix.len()).collect();
                                listener_hits.push(build_listener_hit(
                                    listener,
                                    snapshot,
                                    5_000,
                                    matched_indices,
                                    is_sys,
                                ));
                            }
                        }
                    }
                    if !listener_hits.is_empty() {
                        results.fell_back_to_prefix = true;
                        results.strategy_hint = format!("port :{} (prefix)", port);
                    }
                }
            }

            rank_listeners(&mut listener_hits);
            results.listeners = finalize_group(listener_hits, limit);

            // Process exact PID match equal to port number.
            let mut process_hits: Vec<ProcessHit> = Vec::new();
            let target_pid = port as i32;
            if options.kinds.processes && target_pid != 0 {
                for process in &snapshot.processes {
                    if process.pid == target_pid {
                        let is_sys = is_system_process(process);
                        if is_sys && !options.show_system {
                            continue;
                        }
                        process_hits.push(build_process_hit(process, 10_000, vec![]));
                        break;
                    }
                }
            }
            results.processes = finalize_group(process_hits, limit);
        }
        SearchKind::Pid { pid } => {
            results.strategy_hint = format!("pid {}", pid);
            let matcher = SkimMatcherV2::default();
            let mut process_hits: Vec<ProcessHit> = Vec::new();
            let mut listener_hits: Vec<ListenerHit> = Vec::new();

            if options.kinds.processes {
                for process in &snapshot.processes {
                    if process.pid == pid {
                        let is_sys = is_system_process(process);
                        if is_sys && !options.show_system {
                            continue;
                        }
                        process_hits.push(build_process_hit(process, 10_000, vec![]));
                        break;
                    }
                }
                for process in &snapshot.processes {
                    if process.pid == 0 || process.pid == pid {
                        continue;
                    }
                    let is_sys = is_system_process(process);
                    if is_sys && !options.show_system {
                        continue;
                    }
                    let text = process_search_text(process);
                    if let Some((score, indices)) = matcher.fuzzy_indices(&text, &normalized) {
                        process_hits.push(build_process_hit(process, score, indices));
                    }
                }
            }

            if options.kinds.listeners {
                for listener in &snapshot.listeners {
                    let is_sys = is_system_listener(listener, snapshot);
                    if is_sys && !options.show_system {
                        continue;
                    }
                    let text = listener_search_text(listener, snapshot);
                    if let Some((score, indices)) = matcher.fuzzy_indices(&text, &normalized) {
                        listener_hits.push(build_listener_hit(
                            listener, snapshot, score, indices, is_sys,
                        ));
                    }
                }
            }

            rank_listeners(&mut listener_hits);
            rank_processes(&mut process_hits);
            results.listeners = finalize_group(listener_hits, limit);
            results.processes = finalize_group(process_hits, limit);

            // PID queries also run fuzzy against secondary entities so a
            // PID-shaped string in workload/project/manager metadata still
            // surfaces.
            run_secondary_fuzzy(
                snapshot,
                &matcher,
                &normalized,
                &options,
                limit,
                &mut results,
            );
        }
        SearchKind::Text { ref text } => {
            results.strategy_hint = "text query".to_string();
            let matcher = SkimMatcherV2::default();
            let mut listener_hits: Vec<ListenerHit> = Vec::new();
            let mut process_hits: Vec<ProcessHit> = Vec::new();

            if options.kinds.listeners {
                for listener in &snapshot.listeners {
                    let is_sys = is_system_listener(listener, snapshot);
                    if is_sys && !options.show_system {
                        continue;
                    }
                    let haystack = listener_search_text(listener, snapshot);
                    if let Some((score, indices)) = matcher.fuzzy_indices(&haystack, text) {
                        listener_hits.push(build_listener_hit(
                            listener, snapshot, score, indices, is_sys,
                        ));
                    }
                }
            }

            if options.kinds.processes {
                for process in &snapshot.processes {
                    if process.pid == 0 {
                        continue;
                    }
                    let is_sys = is_system_process(process);
                    if is_sys && !options.show_system {
                        continue;
                    }
                    let haystack = process_search_text(process);
                    if let Some((score, indices)) = matcher.fuzzy_indices(&haystack, text) {
                        process_hits.push(build_process_hit(process, score, indices));
                    }
                }
            }

            rank_listeners(&mut listener_hits);
            rank_processes(&mut process_hits);
            results.listeners = finalize_group(listener_hits, limit);
            results.processes = finalize_group(process_hits, limit);

            run_secondary_fuzzy(snapshot, &matcher, text, &options, limit, &mut results);
        }
    }

    results.elapsed_ms = start.elapsed().as_millis();

    info!(
        query_kind = ?results.query.kind,
        normalized_len = results.query.normalized.len(),
        limit,
        show_system = options.show_system,
        listener_total = results.listeners.total,
        process_total = results.processes.total,
        workload_total = results.workloads.total,
        project_total = results.projects.total,
        manager_total = results.managers.total,
        rail_view_total = results.rail_views.total,
        fell_back_to_prefix = results.fell_back_to_prefix,
        elapsed_ms = results.elapsed_ms,
        "search executed"
    );

    results
}

fn run_secondary_fuzzy(
    snapshot: &Snapshot,
    matcher: &SkimMatcherV2,
    needle: &str,
    options: &SearchOptions,
    limit: usize,
    results: &mut SearchResults,
) {
    if needle.is_empty() {
        return;
    }

    if options.kinds.workloads {
        let mut hits: Vec<WorkloadHit> = Vec::new();
        for workload in &snapshot.workloads {
            let is_sys = is_system_workload(workload, snapshot);
            if is_sys && !options.show_system {
                continue;
            }
            let haystack = workload_search_text(workload, snapshot);
            if let Some((score, indices)) = matcher.fuzzy_indices(&haystack, needle) {
                hits.push(build_workload_hit(workload, snapshot, score, indices));
            }
        }
        rank_workloads(&mut hits);
        results.workloads = finalize_group(hits, limit);
    }

    if options.kinds.projects {
        let mut hits: Vec<ProjectHit> = Vec::new();
        for project in &snapshot.projects {
            let haystack = project_search_text(project);
            if let Some((score, indices)) = matcher.fuzzy_indices(&haystack, needle) {
                hits.push(build_project_hit(project, score, indices));
            }
        }
        rank_projects(&mut hits);
        results.projects = finalize_group(hits, limit);
    }

    if options.kinds.managers {
        let mut hits: Vec<ManagerHit> = Vec::new();
        for manager in &snapshot.managers {
            let is_sys = is_system_manager(manager);
            if is_sys && !options.show_system {
                continue;
            }
            let haystack = manager_search_text(manager);
            if let Some((score, indices)) = matcher.fuzzy_indices(&haystack, needle) {
                hits.push(build_manager_hit(manager, score, indices));
            }
        }
        rank_managers(&mut hits);
        results.managers = finalize_group(hits, limit);
    }

    if options.kinds.rail_views {
        let mut hits: Vec<RailViewHit> = Vec::new();
        for entry in RAIL_ENTRIES {
            // Match against id + label so both "listeners" (id) and
            // "Listeners" (label) hit on the same query.
            let haystack = format!("{} {}", entry.id, entry.label);
            if let Some((score, indices)) = matcher.fuzzy_indices(&haystack, needle) {
                hits.push(RailViewHit {
                    id: entry.id.to_string(),
                    label: entry.label.to_string(),
                    score,
                    matched_indices: indices,
                });
            }
        }
        rank_rail_views(&mut hits);
        results.rail_views = finalize_group(hits, limit);
    }
}

// ---------------------------------------------------------------------------
// Selection helpers for TUI flat indexing.
// ---------------------------------------------------------------------------

pub enum SearchHitRef<'a> {
    Listener(&'a ListenerHit),
    Process(&'a ProcessHit),
    Workload(&'a WorkloadHit),
    Project(&'a ProjectHit),
    Manager(&'a ManagerHit),
    RailView(&'a RailViewHit),
}

impl SearchHitRef<'_> {
    pub fn score(&self) -> i64 {
        match self {
            SearchHitRef::Listener(hit) => hit.score,
            SearchHitRef::Process(hit) => hit.score,
            SearchHitRef::Workload(hit) => hit.score,
            SearchHitRef::Project(hit) => hit.score,
            SearchHitRef::Manager(hit) => hit.score,
            SearchHitRef::RailView(hit) => hit.score,
        }
    }

    fn kind_rank(&self) -> usize {
        match self {
            SearchHitRef::Project(_) => 0,
            SearchHitRef::Workload(_) => 1,
            SearchHitRef::Process(_) => 2,
            SearchHitRef::Listener(_) => 3,
            SearchHitRef::Manager(_) => 4,
            SearchHitRef::RailView(_) => 5,
        }
    }

    fn stable_key(&self) -> String {
        match self {
            SearchHitRef::Listener(hit) => format!("listener:{}", hit.id),
            SearchHitRef::Process(hit) => format!("process:{:010}", hit.pid),
            SearchHitRef::Workload(hit) => format!("workload:{}", hit.display_name),
            SearchHitRef::Project(hit) => format!("project:{}", hit.name),
            SearchHitRef::Manager(hit) => format!("manager:{}", hit.name),
            SearchHitRef::RailView(hit) => format!("rail:{}", hit.label),
        }
    }
}

pub fn ranked_search_hits(results: &SearchResults) -> Vec<SearchHitRef<'_>> {
    let mut hits = Vec::with_capacity(search_hit_count(results));
    hits.extend(results.projects.hits.iter().map(SearchHitRef::Project));
    hits.extend(results.workloads.hits.iter().map(SearchHitRef::Workload));
    hits.extend(results.processes.hits.iter().map(SearchHitRef::Process));
    hits.extend(results.listeners.hits.iter().map(SearchHitRef::Listener));
    hits.extend(results.managers.hits.iter().map(SearchHitRef::Manager));
    hits.extend(results.rail_views.hits.iter().map(SearchHitRef::RailView));
    hits.sort_by(|a, b| {
        b.score()
            .cmp(&a.score())
            .then_with(|| a.kind_rank().cmp(&b.kind_rank()))
            .then_with(|| a.stable_key().cmp(&b.stable_key()))
    });
    hits
}

pub fn search_hit_count(results: &SearchResults) -> usize {
    results.listeners.hits.len()
        + results.processes.hits.len()
        + results.workloads.hits.len()
        + results.projects.hits.len()
        + results.managers.hits.len()
        + results.rail_views.hits.len()
}

pub fn search_hit_at(results: &SearchResults, flat_index: usize) -> Option<SearchHitRef<'_>> {
    ranked_search_hits(results).into_iter().nth(flat_index)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_empty() {
        assert_eq!(classify(""), SearchKind::Empty);
        assert_eq!(classify("   "), SearchKind::Empty);
    }

    #[test]
    fn classify_text() {
        assert_eq!(
            classify("hermes"),
            SearchKind::Text {
                text: "hermes".into()
            }
        );
        assert_eq!(classify("-1"), SearchKind::Text { text: "-1".into() });
    }

    #[test]
    fn classify_port() {
        assert_eq!(classify("5432"), SearchKind::Port { port: 5432 });
        assert_eq!(classify("54"), SearchKind::Port { port: 54 });
        assert_eq!(classify("65535"), SearchKind::Port { port: 65535 });
    }

    #[test]
    fn classify_pid() {
        assert_eq!(classify("99999"), SearchKind::Pid { pid: 99999 });
        assert_eq!(classify("1234567"), SearchKind::Pid { pid: 1234567 });
    }

    #[test]
    fn classify_overflow() {
        let big = "99999999999999999999";
        assert_eq!(classify(big), SearchKind::Text { text: big.into() });
    }

    #[test]
    fn classify_zero_and_negative_pid() {
        assert_eq!(classify("0"), SearchKind::Port { port: 0 });
        assert_eq!(classify("-1"), SearchKind::Text { text: "-1".into() });
    }

    #[test]
    fn search_results_default() {
        let r = SearchResults::default();
        assert_eq!(r.schema_version, SEARCH_SCHEMA_VERSION);
        assert_eq!(r.query.kind, SearchKind::Empty);
        assert!(r.listeners.hits.is_empty());
        assert!(r.processes.hits.is_empty());
        assert!(r.workloads.hits.is_empty());
        assert!(r.projects.hits.is_empty());
        assert!(r.managers.hits.is_empty());
        assert!(r.rail_views.hits.is_empty());
        assert_eq!(r.strategy_hint, "");
        assert!(!r.fell_back_to_prefix);
        assert_eq!(r.elapsed_ms, 0);
    }

    #[test]
    fn empty_query_returns_no_hits() {
        let snapshot = Snapshot::empty();
        let r = run(&snapshot, "", SearchOptions::default());
        assert_eq!(r.query.kind, SearchKind::Empty);
        assert_eq!(search_hit_count(&r), 0);
        assert_eq!(r.strategy_hint, "");
    }

    #[test]
    fn rail_views_match_text() {
        let snapshot = Snapshot::empty();
        let r = run(&snapshot, "listen", SearchOptions::default());
        // The rail entry "Listeners" / "listeners" should fuzzy match "listen".
        assert!(r.rail_views.total >= 1);
        assert!(r.rail_views.hits.iter().any(|h| h.id == "listeners"));
    }

    #[test]
    fn kinds_filter_disables_groups() {
        let snapshot = Snapshot::empty();
        let opts = SearchOptions {
            kinds: SearchKinds {
                rail_views: false,
                ..SearchKinds::default()
            },
            ..SearchOptions::default()
        };
        let r = run(&snapshot, "listen", opts);
        assert_eq!(r.rail_views.total, 0);
    }

    #[test]
    fn ranked_hits_prioritize_actionable_entities_on_score_ties() {
        let mut results = SearchResults::default();
        results.listeners.hits.push(ListenerHit {
            id: ListenerId::new("unix::0:1"),
            port: None,
            bind: "/tmp/codex.sock".into(),
            protocol: Protocol::Unix,
            exposure: Exposure::UnixLocal,
            owner_label: "/opt/codex".into(),
            workload_labels: Vec::new(),
            project_label: None,
            score: 100,
            matched_indices: Vec::new(),
            is_system: false,
        });
        results.processes.hits.push(ProcessHit {
            key: ProcessKey {
                pid: 42,
                boot_id: "boot".into(),
                start_time_ticks: 1,
            },
            pid: 42,
            user: Some("shuv".into()),
            exe_or_argv0: "codex".into(),
            cmdline_compact: "codex".into(),
            cwd: Some(PathBuf::from("/home/shuv/repos/lazyadmin")),
            score: 100,
            matched_indices: Vec::new(),
            is_system: false,
        });

        assert!(matches!(
            search_hit_at(&results, 0),
            Some(SearchHitRef::Process(hit)) if hit.pid == 42
        ));
        assert!(matches!(
            search_hit_at(&results, 1),
            Some(SearchHitRef::Listener(hit)) if hit.id == ListenerId::new("unix::0:1")
        ));
    }
}
