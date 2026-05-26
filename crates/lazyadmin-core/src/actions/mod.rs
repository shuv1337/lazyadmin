#![allow(clippy::module_name_repetitions)]

use crate::model::{
    ActionId, DangerLevel, EdgeKind, EntityRef, Listener, Process, ProcessKey, RuntimeKind,
    Snapshot, Workload,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Stop,
    Restart,
    Kill,
    FreePort,
    PortlessStop,
    PauseRestart,
    ResumeRestart,
    Logs,
    Forget,
    SignalProcessGroup,
    SignalPid,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Requirement {
    RuntimeAvailable { runtime: RuntimeKind },
    Permission { detail: String },
    PolkitOrSudo { detail: String },
    TypedPhrase { phrase: String },
    SelectorDisambiguation,
    RestartPolicyPauseRecommended { policy: String },
    ProcessKeyMatch { key: ProcessKey },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ConfirmationPolicy {
    None,
    YesNo,
    TypedPhrase { phrase: String },
    TestOnlyBypass,
}

impl ConfirmationPolicy {
    #[must_use]
    pub fn render_prompt(&self) -> String {
        match self {
            Self::None => "no confirmation required".into(),
            Self::YesNo => "Continue? [y/N]".into(),
            Self::TypedPhrase { phrase } => format!("Type \"{phrase}\" to continue:"),
            Self::TestOnlyBypass => "TEST-ONLY confirmation bypass in effect".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DryRunLine {
    pub summary: String,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub id: ActionId,
    pub label: String,
    pub kind: ActionKind,
    pub danger: DangerLevel,
    pub requirements: Vec<Requirement>,
    pub dry_run: Vec<DryRunLine>,
    pub target: EntityRef,
    pub runtime: RuntimeKind,
    pub confirmation: ConfirmationPolicy,
    pub timeout_ms: u64,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionPlan {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub target: String,
    pub actions: Vec<Action>,
    pub dry_run: Vec<DryRunLine>,
    pub confirmation: ConfirmationPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    Success,
    Failed,
    TimedOut,
    Skipped,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionResult {
    pub action_id: ActionId,
    pub status: ActionStatus,
    pub message: String,
    pub duration_ms: u128,
    pub error_class: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionExecutionReport {
    pub schema_version: String,
    pub plan: ActionPlan,
    pub results: Vec<ActionResult>,
    pub before_summary: String,
    pub after_summary: String,
    pub diff_summaries: Vec<String>,
}

pub trait ActionPlanner {
    fn plan(&self, target: &EntityRef, graph: &Snapshot) -> Vec<Action>;
}

pub trait ActionExecutor {
    fn execute(&self, action: &Action, timeout: Duration) -> ActionResult;
}

#[derive(Clone, Debug, Default)]
pub struct ActionPlanTelemetry {
    pub action_count: usize,
}

pub fn render_dry_run(lines: &[DryRunLine]) -> String {
    lines
        .iter()
        .map(|l| match &l.detail {
            Some(d) => format!("- {} ({d})", l.summary),
            None => format!("- {}", l.summary),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

pub fn first_dry_run_line(lines: &[DryRunLine]) -> Option<String> {
    render_dry_run(lines).lines().next().map(str::to_string)
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct FreePortPlan {
    pub listeners: Vec<Listener>,
    pub portless_actions: Vec<Action>,
    pub direct_actions: Vec<Action>,
}

impl FreePortPlan {
    #[must_use]
    pub fn actions(&self) -> Vec<Action> {
        self.portless_actions
            .iter()
            .chain(self.direct_actions.iter())
            .cloned()
            .collect()
    }

    #[must_use]
    pub fn dry_run(&self, port: u16) -> Vec<DryRunLine> {
        free_port_dry_run(port, &self.listeners, &self.actions())
    }
}

#[must_use]
pub fn plan_free_port_for_snapshot(
    snap: &Snapshot,
    port: u16,
    include_portless: bool,
) -> FreePortPlan {
    let listeners: Vec<_> = snap
        .listeners
        .iter()
        .filter(|listener| listener.port == Some(port))
        .cloned()
        .collect();
    let mut parts = FreePortPlan {
        listeners,
        ..FreePortPlan::default()
    };
    let mut planned_processes = std::collections::HashSet::new();
    let mut planned_portless = std::collections::BTreeSet::new();
    for listener in &parts.listeners {
        let portless_workloads = portless_workloads_for_listener(snap, listener);
        if include_portless {
            for workload in &portless_workloads {
                if planned_portless.insert(workload.id.clone()) {
                    if let Some(action) = plan_portless_stop(workload, port) {
                        parts.portless_actions.push(action);
                    }
                }
            }
        }
        if !portless_workloads.is_empty() {
            continue;
        }
        for owner in &listener.owners {
            if let EntityRef::Process(key) = owner {
                if planned_processes.insert(key.clone()) {
                    if let Some(process) = snap.processes.iter().find(|process| &process.key == key)
                    {
                        parts
                            .direct_actions
                            .push(plan_direct_process_free_port(process, port));
                    }
                }
            }
        }
    }
    if parts.listeners.is_empty() && include_portless {
        let needle = port.to_string();
        for process in snap.processes.iter().filter(|process| {
            process.cmdline.iter().any(|arg| arg == &needle)
                && process
                    .cmdline
                    .iter()
                    .any(|arg| arg.contains("http.server"))
        }) {
            if planned_processes.insert(process.key.clone()) {
                parts
                    .direct_actions
                    .push(plan_direct_process_free_port(process, port));
            }
        }
    }
    parts
}

fn portless_workloads_for_listener<'a>(
    snap: &'a Snapshot,
    listener: &Listener,
) -> Vec<&'a Workload> {
    let listener_ref = EntityRef::Listener(listener.id.clone());
    snap.edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::WorkloadOwnsListener && edge.to == listener_ref)
        .filter_map(|edge| match &edge.from {
            EntityRef::Workload(id) => snap
                .workloads
                .iter()
                .find(|workload| &workload.id == id && workload.runtime == RuntimeKind::Portless),
            _ => None,
        })
        .collect()
}

fn plan_portless_stop(workload: &Workload, port: u16) -> Option<Action> {
    let Some(EntityRef::Process(key)) = &workload.source else {
        return None;
    };
    Some(Action {
        id: ActionId::new(format!("portless-stop-{}", workload.id)),
        label: format!("Stop portless app {}", workload.display_name),
        kind: ActionKind::PortlessStop,
        danger: DangerLevel::Destructive,
        requirements: vec![
            Requirement::ProcessKeyMatch { key: key.clone() },
            Requirement::TypedPhrase {
                phrase: "free".into(),
            },
        ],
        dry_run: vec![DryRunLine {
            summary: format!(
                "stop portless app \"{}\" (manager: portless)",
                workload.display_name
            ),
            detail: Some(format!(
                "SIGTERM PID {} (portless cli); portless will killTree the dev-server and remove the route for port {port}",
                key.pid
            )),
        }],
        target: EntityRef::Process(key.clone()),
        runtime: RuntimeKind::Portless,
        confirmation: ConfirmationPolicy::TypedPhrase {
            phrase: "free".into(),
        },
        timeout_ms: 5_000,
        provenance: vec![format!("portless workload {}", workload.id)],
    })
}

#[must_use]
pub fn plan_direct_process_free_port(process: &Process, port: u16) -> Action {
    let pgid = process.pgid.unwrap_or(process.pid);
    let use_group = pgid == process.pid;
    Action {
        id: ActionId::new(format!(
            "signal-{}-{}",
            if use_group { "pgrp" } else { "pid" },
            process.pid
        )),
        label: if use_group {
            format!("Send SIGTERM to process group {pgid}")
        } else {
            format!("Send SIGTERM to PID {}", process.pid)
        },
        kind: if use_group {
            ActionKind::SignalProcessGroup
        } else {
            ActionKind::SignalPid
        },
        danger: DangerLevel::Destructive,
        requirements: vec![
            Requirement::ProcessKeyMatch {
                key: process.key.clone(),
            },
            Requirement::TypedPhrase {
                phrase: "free".into(),
            },
        ],
        dry_run: vec![DryRunLine {
            summary: format!(
                "stop PID {} ({})",
                process.pid,
                process
                    .cmdline
                    .first()
                    .cloned()
                    .unwrap_or_else(|| "process".into())
            ),
            detail: Some(format!(
                "SIGTERM {}; port {port} expected to disappear; SIGKILL will not be used automatically",
                if use_group {
                    format!("process group {pgid}")
                } else {
                    format!("PID {}", process.pid)
                }
            )),
        }],
        target: EntityRef::Process(process.key.clone()),
        runtime: RuntimeKind::Direct,
        confirmation: ConfirmationPolicy::TypedPhrase {
            phrase: "free".into(),
        },
        timeout_ms: 5_000,
        provenance: vec!["procfs listener owner".into()],
    }
}

#[must_use]
pub fn free_port_dry_run(port: u16, listeners: &[Listener], actions: &[Action]) -> Vec<DryRunLine> {
    let mut lines = vec![DryRunLine {
        summary: format!(
            "free port {port}: {} listener(s), {} owner action(s)",
            listeners.len(),
            actions.len()
        ),
        detail: Some(
            "one consolidated confirmation; portless routes are stopped through their CLI, direct owners use process-key guarded SIGTERM"
                .into(),
        ),
    }];
    for action in actions {
        lines.extend(action.dry_run.clone());
    }
    lines.push(DryRunLine {
        summary: "will not touch unrelated ports or use SIGKILL automatically".into(),
        detail: None,
    });
    lines
}

#[must_use]
pub fn plan_free_port_preview_action(snap: &Snapshot, port: u16, include_portless: bool) -> Action {
    let plan = plan_free_port_for_snapshot(snap, port, include_portless);
    let actions = plan.actions();
    let target = plan
        .listeners
        .first()
        .map(|listener| EntityRef::Listener(listener.id.clone()))
        .unwrap_or_else(|| EntityRef::Action(ActionId::new(format!("free-port-preview-{port}"))));
    Action {
        id: ActionId::new(format!("free-port-preview-{port}")),
        label: format!("Preview free port {port}"),
        kind: ActionKind::FreePort,
        danger: DangerLevel::Destructive,
        requirements: vec![Requirement::TypedPhrase {
            phrase: "free".into(),
        }],
        dry_run: free_port_dry_run(port, &plan.listeners, &actions),
        target,
        runtime: RuntimeKind::Direct,
        confirmation: ConfirmationPolicy::TypedPhrase {
            phrase: "free".into(),
        },
        timeout_ms: 5_000,
        provenance: vec!["free-port planner preview".into()],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{
        AddressFamily, Confidence, DualStackState, Edge, EdgeKind, Exposure, ListenerId,
        ListenerState, RedactedEnvironmentSummary, WorkloadId, WorkloadState,
    };
    #[test]
    fn confirmation_policy_rendering() {
        assert!(
            ConfirmationPolicy::TypedPhrase {
                phrase: "free".into()
            }
            .render_prompt()
            .contains("free")
        );
    }
    #[test]
    fn dry_run_stable_output() {
        assert_eq!(
            render_dry_run(&[DryRunLine {
                summary: "stop pid".into(),
                detail: Some("SIGTERM".into())
            }]),
            "- stop pid (SIGTERM)"
        );
    }
    #[test]
    fn action_serialization() {
        let p = ActionPlan {
            id: "p".into(),
            created_at: Utc::now(),
            target: ":1".into(),
            actions: vec![],
            dry_run: vec![],
            confirmation: ConfirmationPolicy::YesNo,
        };
        serde_json::to_string(&p).unwrap();
    }

    #[test]
    fn confirmation_none_yes_no_and_bypass_render_distinct_prompts() {
        let none = ConfirmationPolicy::None.render_prompt();
        let yn = ConfirmationPolicy::YesNo.render_prompt();
        let bypass = ConfirmationPolicy::TestOnlyBypass.render_prompt();
        assert_ne!(none, yn);
        assert_ne!(yn, bypass);
        assert!(none.contains("no confirmation"));
        assert!(yn.contains("[y/N]"));
        assert!(bypass.contains("TEST-ONLY"));
    }

    #[test]
    fn typed_phrase_prompt_quotes_the_phrase() {
        let p = ConfirmationPolicy::TypedPhrase {
            phrase: "DELETE".into(),
        };
        let s = p.render_prompt();
        assert!(s.contains("\"DELETE\""));
    }

    #[test]
    fn render_dry_run_handles_lines_without_detail() {
        let out = render_dry_run(&[
            DryRunLine {
                summary: "plan a".into(),
                detail: None,
            },
            DryRunLine {
                summary: "plan b".into(),
                detail: Some("SIGKILL".into()),
            },
        ]);
        assert_eq!(out, "- plan a\n- plan b (SIGKILL)");
    }

    #[test]
    fn render_dry_run_empty_returns_empty_string() {
        assert_eq!(render_dry_run(&[]), "");
    }

    #[test]
    fn free_port_planner_prefers_portless_cli_over_descendant_owner() {
        let cli = process_key(100, 10);
        let child = process_key(101, 11);
        let mut snap = Snapshot::empty();
        snap.processes
            .push(process(cli.clone(), None, vec!["portless"]));
        snap.processes
            .push(process(child.clone(), Some(100), vec!["node"]));
        let listener_id = ListenerId::new("tcp:127.0.0.1:3737:1");
        snap.listeners.push(listener(
            listener_id.clone(),
            3737,
            vec![EntityRef::Process(child)],
        ));
        let workload_id = WorkloadId::new("portless:demo");
        snap.workloads.push(portless_workload(
            workload_id.clone(),
            "demo.localhost",
            Some(EntityRef::Process(cli.clone())),
            vec![listener_id.clone()],
        ));
        snap.edges.push(Edge {
            kind: EdgeKind::WorkloadOwnsListener,
            from: EntityRef::Workload(workload_id),
            to: EntityRef::Listener(listener_id),
            provenance: vec![],
        });

        let plan = plan_free_port_for_snapshot(&snap, 3737, true);
        assert_eq!(plan.portless_actions.len(), 1);
        assert!(plan.direct_actions.is_empty());
        assert_eq!(plan.portless_actions[0].kind, ActionKind::PortlessStop);
        assert_eq!(plan.portless_actions[0].target, EntityRef::Process(cli));
    }

    #[test]
    fn free_port_planner_handles_direct_and_mixed_ports() {
        let direct = process_key(200, 20);
        let cli = process_key(300, 30);
        let child = process_key(301, 31);
        let mut snap = Snapshot::empty();
        snap.processes
            .push(process(direct.clone(), None, vec!["python"]));
        snap.processes
            .push(process(cli.clone(), None, vec!["portless"]));
        snap.processes
            .push(process(child.clone(), Some(300), vec!["node"]));
        let direct_listener = ListenerId::new("tcp:127.0.0.1:8080:1");
        let portless_listener = ListenerId::new("tcp:127.0.0.1:8080:2");
        snap.listeners.push(listener(
            direct_listener,
            8080,
            vec![EntityRef::Process(direct.clone())],
        ));
        snap.listeners.push(listener(
            portless_listener.clone(),
            8080,
            vec![EntityRef::Process(child)],
        ));
        let workload_id = WorkloadId::new("portless:mixed");
        snap.workloads.push(portless_workload(
            workload_id.clone(),
            "mixed.localhost",
            Some(EntityRef::Process(cli.clone())),
            vec![portless_listener.clone()],
        ));
        snap.edges.push(Edge {
            kind: EdgeKind::WorkloadOwnsListener,
            from: EntityRef::Workload(workload_id),
            to: EntityRef::Listener(portless_listener),
            provenance: vec![],
        });

        let plan = plan_free_port_for_snapshot(&snap, 8080, true);
        assert_eq!(plan.portless_actions.len(), 1);
        assert_eq!(plan.direct_actions.len(), 1);
        assert_eq!(plan.portless_actions[0].target, EntityRef::Process(cli));
        assert_eq!(plan.direct_actions[0].target, EntityRef::Process(direct));
    }

    #[test]
    fn free_port_planner_dedupes_same_portless_workload() {
        let cli = process_key(400, 40);
        let child = process_key(401, 41);
        let mut snap = Snapshot::empty();
        snap.processes
            .push(process(cli.clone(), None, vec!["portless"]));
        snap.processes
            .push(process(child.clone(), Some(400), vec!["node"]));
        let workload_id = WorkloadId::new("portless:dedupe");
        snap.workloads.push(portless_workload(
            workload_id.clone(),
            "dedupe.localhost",
            Some(EntityRef::Process(cli)),
            vec![],
        ));
        for suffix in [1, 2] {
            let listener_id = ListenerId::new(format!("tcp:127.0.0.1:9090:{suffix}"));
            snap.listeners.push(listener(
                listener_id.clone(),
                9090,
                vec![EntityRef::Process(child.clone())],
            ));
            snap.edges.push(Edge {
                kind: EdgeKind::WorkloadOwnsListener,
                from: EntityRef::Workload(workload_id.clone()),
                to: EntityRef::Listener(listener_id),
                provenance: vec![],
            });
        }

        let plan = plan_free_port_for_snapshot(&snap, 9090, true);
        assert_eq!(plan.portless_actions.len(), 1);
        assert!(plan.direct_actions.is_empty());
    }

    #[test]
    fn free_port_planner_refuses_portless_without_source_and_ignores_alias_without_listener() {
        let child = process_key(501, 51);
        let mut snap = Snapshot::empty();
        snap.processes
            .push(process(child.clone(), None, vec!["node"]));
        let listener_id = ListenerId::new("tcp:127.0.0.1:6060:1");
        snap.listeners.push(listener(
            listener_id.clone(),
            6060,
            vec![EntityRef::Process(child)],
        ));
        let workload_id = WorkloadId::new("portless:missing-source");
        snap.workloads.push(portless_workload(
            workload_id.clone(),
            "missing-source.localhost",
            None,
            vec![listener_id.clone()],
        ));
        snap.workloads.push(portless_workload(
            WorkloadId::new("portless:alias"),
            "alias.localhost",
            None,
            vec![],
        ));
        snap.edges.push(Edge {
            kind: EdgeKind::WorkloadOwnsListener,
            from: EntityRef::Workload(workload_id),
            to: EntityRef::Listener(listener_id),
            provenance: vec![],
        });

        let plan = plan_free_port_for_snapshot(&snap, 6060, true);
        assert!(plan.portless_actions.is_empty());
        assert!(plan.direct_actions.is_empty());
    }

    #[test]
    fn action_kind_round_trips_via_json_snake_case() {
        let cases = [
            (ActionKind::Stop, "stop"),
            (ActionKind::Restart, "restart"),
            (ActionKind::Kill, "kill"),
            (ActionKind::FreePort, "free_port"),
            (ActionKind::PortlessStop, "portless_stop"),
            (ActionKind::PauseRestart, "pause_restart"),
            (ActionKind::ResumeRestart, "resume_restart"),
            (ActionKind::Logs, "logs"),
            (ActionKind::Forget, "forget"),
            (ActionKind::SignalProcessGroup, "signal_process_group"),
            (ActionKind::SignalPid, "signal_pid"),
            (ActionKind::Unsupported, "unsupported"),
        ];
        for (kind, expected) in cases {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{expected}\""));
            let back: ActionKind = serde_json::from_str(&json).unwrap();
            assert_eq!(back, kind);
        }
    }

    fn process_key(pid: i32, start_time_ticks: u64) -> ProcessKey {
        ProcessKey {
            pid,
            boot_id: "boot".into(),
            start_time_ticks,
        }
    }

    fn process(key: ProcessKey, ppid: Option<i32>, cmdline: Vec<&str>) -> Process {
        Process {
            pid: key.pid,
            start_time_ticks: key.start_time_ticks,
            boot_id: key.boot_id.clone(),
            key,
            user: None,
            exe: None,
            cmdline: cmdline.into_iter().map(str::to_string).collect(),
            cwd: None,
            ppid,
            pgid: None,
            sid: None,
            cgroup: None,
            netns: None,
            container_id: None,
            systemd_unit: None,
            lazyadmin_run_id: None,
            environment: RedactedEnvironmentSummary::default(),
            provenance: vec![],
        }
    }

    fn listener(id: ListenerId, port: u16, owners: Vec<EntityRef>) -> Listener {
        Listener {
            id,
            protocol: crate::model::Protocol::Tcp,
            family: AddressFamily::Ipv4,
            bind_addr: Some("127.0.0.1".into()),
            port: Some(port),
            path: None,
            state: ListenerState::Listen,
            netns: "host".into(),
            socket_inode: None,
            exposure: Exposure::Loopback,
            owners,
            confidence: Confidence::High,
            provenance: vec![],
            first_seen: Utc::now(),
            last_seen: Utc::now(),
            dual_stack_state: DualStackState::NotApplicable,
        }
    }

    fn portless_workload(
        id: WorkloadId,
        display_name: &str,
        source: Option<EntityRef>,
        listeners: Vec<ListenerId>,
    ) -> Workload {
        Workload {
            id,
            display_name: display_name.into(),
            runtime: RuntimeKind::Portless,
            state: WorkloadState::Running,
            pids: vec![],
            listeners,
            project: None,
            manager: None,
            source,
            actions: vec![],
            health: None,
            metrics: None,
            restart_policy: None,
            lazyadmin_run_id: None,
            provenance: vec![],
        }
    }

    #[test]
    fn action_status_round_trips_via_json_snake_case() {
        for s in [
            ActionStatus::Success,
            ActionStatus::Failed,
            ActionStatus::TimedOut,
            ActionStatus::Skipped,
            ActionStatus::Unsupported,
        ] {
            let json = serde_json::to_string(&s).unwrap();
            let back: ActionStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(back, s);
        }
    }

    #[test]
    fn requirement_serializes_with_kind_and_detail() {
        let r = Requirement::Permission {
            detail: "need CAP_SYS_PTRACE".into(),
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: Requirement = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn action_plan_telemetry_default_is_zero() {
        let t = ActionPlanTelemetry::default();
        assert_eq!(t.action_count, 0);
    }

    #[test]
    fn action_result_round_trips_with_error_class() {
        let r = ActionResult {
            action_id: ActionId::new("a-1"),
            status: ActionStatus::Failed,
            message: "timed out".into(),
            duration_ms: 1500,
            error_class: Some("io::ErrorKind::TimedOut".into()),
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: ActionResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }
}
