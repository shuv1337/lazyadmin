use lazyadmin_core::model::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InspectorView {
    Listener {
        id: ListenerId,
        title: String,
        facts: Vec<(String, String)>,
    },
    Workload {
        id: WorkloadId,
        title: String,
        facts: Vec<(String, String)>,
    },
    Process {
        key: ProcessKey,
        title: String,
        facts: Vec<(String, String)>,
    },
    Project {
        id: ProjectId,
        title: String,
        facts: Vec<(String, String)>,
    },
    Manager {
        id: ManagerId,
        title: String,
        facts: Vec<(String, String)>,
    },
    TrackedRun {
        id: RunId,
        title: String,
        facts: Vec<(String, String)>,
    },
    WarningGroup {
        code: String,
        title: String,
        facts: Vec<(String, String)>,
    },
}

impl InspectorView {
    /// Locate a single inspector view by `kind` + `id`. Used by the
    /// read-only Web UI's `/api/inspector` endpoint. `kind` accepts both
    /// singular and plural spellings (`listener`/`listeners`). For
    /// processes, `id` may be either the JSON-encoded `ProcessKey` or the
    /// bare PID string. Warning groups look up by `code`.
    pub fn lookup(snapshot: &Snapshot, kind: &str, id: &str) -> Option<Self> {
        match kind {
            "listener" | "listeners" => snapshot
                .listeners
                .iter()
                .find(|listener| listener.id.to_string() == id)
                .map(|listener| Self::Listener {
                    id: listener.id.clone(),
                    title: listener_label(listener),
                    facts: vec![
                        ("protocol".into(), format!("{:?}", listener.protocol)),
                        ("exposure".into(), format!("{:?}", listener.exposure)),
                        ("owners".into(), listener.owners.len().to_string()),
                    ],
                }),
            "workload" | "workloads" => snapshot
                .workloads
                .iter()
                .find(|workload| workload.id.to_string() == id)
                .map(|workload| Self::Workload {
                    id: workload.id.clone(),
                    title: workload.display_name.clone(),
                    facts: vec![
                        ("runtime".into(), format!("{:?}", workload.runtime)),
                        ("state".into(), format!("{:?}", workload.state)),
                        ("listeners".into(), workload.listeners.len().to_string()),
                    ],
                }),
            "process" | "processes" => snapshot
                .processes
                .iter()
                .find(|process| {
                    serde_json::to_string(&process.key)
                        .ok()
                        .is_some_and(|k| k == id)
                        || process.pid.to_string() == id
                })
                .map(|process| Self::Process {
                    key: process.key.clone(),
                    title: process
                        .cmdline
                        .first()
                        .cloned()
                        .unwrap_or_else(|| format!("pid {}", process.pid)),
                    facts: vec![
                        ("pid".into(), process.pid.to_string()),
                        (
                            "user".into(),
                            process.user.clone().unwrap_or_else(|| "unknown".into()),
                        ),
                    ],
                }),
            "project" | "projects" => snapshot
                .projects
                .iter()
                .find(|project| project.id.to_string() == id)
                .map(|project| Self::Project {
                    id: project.id.clone(),
                    title: project.name.clone(),
                    facts: vec![
                        ("root".into(), project.root.display().to_string()),
                        ("markers".into(), project.markers.len().to_string()),
                    ],
                }),
            "manager" | "managers" => snapshot
                .managers
                .iter()
                .find(|manager| manager.id.to_string() == id)
                .map(|manager| Self::Manager {
                    id: manager.id.clone(),
                    title: manager.name.clone(),
                    facts: vec![
                        ("kind".into(), format!("{:?}", manager.kind)),
                        ("available".into(), manager.available.to_string()),
                    ],
                }),
            "run" | "runs" | "tracked_run" | "tracked_runs" => snapshot
                .tracked_runs
                .iter()
                .find(|run| run.id.to_string() == id)
                .map(|run| Self::TrackedRun {
                    id: run.id.clone(),
                    title: run.tag.clone().unwrap_or_else(|| run.command.join(" ")),
                    facts: vec![
                        ("state".into(), format!("{:?}", run.state)),
                        (
                            "cwd".into(),
                            run.cwd
                                .as_ref()
                                .map(|p| p.display().to_string())
                                .unwrap_or_else(|| "unknown".into()),
                        ),
                    ],
                }),
            "warning" | "warning_group" | "warning_groups" => {
                let count = snapshot
                    .warnings
                    .iter()
                    .filter(|warning| warning.code == id)
                    .count();
                if count == 0 {
                    return None;
                }
                let severities: std::collections::BTreeSet<_> = snapshot
                    .warnings
                    .iter()
                    .filter(|warning| warning.code == id)
                    .map(|warning| format!("{:?}", warning.severity))
                    .collect();
                Some(Self::WarningGroup {
                    code: id.to_string(),
                    title: format!("warning {id}"),
                    facts: vec![
                        ("count".into(), count.to_string()),
                        (
                            "severities".into(),
                            severities.into_iter().collect::<Vec<_>>().join(", "),
                        ),
                    ],
                })
            }
            _ => None,
        }
    }

    pub fn all_from_snapshot(snapshot: &Snapshot) -> Vec<Self> {
        let mut views = Vec::new();
        views.extend(snapshot.listeners.iter().map(|listener| Self::Listener {
            id: listener.id.clone(),
            title: listener_label(listener),
            facts: vec![
                ("protocol".into(), format!("{:?}", listener.protocol)),
                ("exposure".into(), format!("{:?}", listener.exposure)),
                ("owners".into(), listener.owners.len().to_string()),
            ],
        }));
        views.extend(snapshot.workloads.iter().map(|workload| Self::Workload {
            id: workload.id.clone(),
            title: workload.display_name.clone(),
            facts: vec![
                ("runtime".into(), format!("{:?}", workload.runtime)),
                ("state".into(), format!("{:?}", workload.state)),
                ("listeners".into(), workload.listeners.len().to_string()),
            ],
        }));
        views.extend(snapshot.processes.iter().map(|process| {
            Self::Process {
                key: process.key.clone(),
                title: process
                    .cmdline
                    .first()
                    .cloned()
                    .unwrap_or_else(|| format!("pid {}", process.pid)),
                facts: vec![
                    ("pid".into(), process.pid.to_string()),
                    (
                        "user".into(),
                        process.user.clone().unwrap_or_else(|| "unknown".into()),
                    ),
                ],
            }
        }));
        views.extend(snapshot.projects.iter().map(|project| Self::Project {
            id: project.id.clone(),
            title: project.name.clone(),
            facts: vec![
                ("root".into(), project.root.display().to_string()),
                ("markers".into(), project.markers.len().to_string()),
            ],
        }));
        views.extend(snapshot.managers.iter().map(|manager| Self::Manager {
            id: manager.id.clone(),
            title: manager.name.clone(),
            facts: vec![
                ("kind".into(), format!("{:?}", manager.kind)),
                ("available".into(), manager.available.to_string()),
            ],
        }));
        views.extend(snapshot.tracked_runs.iter().map(|run| Self::TrackedRun {
            id: run.id.clone(),
            title: run.tag.clone().unwrap_or_else(|| run.command.join(" ")),
            facts: vec![
                    ("state".into(), format!("{:?}", run.state)),
                    (
                        "cwd".into(),
                        run.cwd
                            .as_ref()
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|| "unknown".into()),
                    ),
                ],
        }));
        views
    }
}

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
