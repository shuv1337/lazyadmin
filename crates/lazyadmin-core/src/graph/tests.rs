use super::*;
use chrono::Utc;

fn prov(adapter: &str, confidence: Confidence) -> Provenance {
    Provenance {
        adapter: adapter.into(),
        claim: "test".into(),
        evidence: "test".into(),
        confidence,
        timestamp: Utc::now(),
    }
}

fn listener(id: &str, confidence: Confidence, provs: Vec<Provenance>) -> Listener {
    Listener {
        id: ListenerId::new(id),
        protocol: Protocol::Tcp,
        family: AddressFamily::Ipv4,
        bind_addr: Some("127.0.0.1".into()),
        port: Some(3000),
        path: None,
        state: ListenerState::Listen,
        netns: "host".into(),
        socket_inode: None,
        exposure: Exposure::Loopback,
        owners: vec![],
        confidence,
        provenance: provs,
        first_seen: Utc::now(),
        last_seen: Utc::now(),
        dual_stack_state: DualStackState::NotApplicable,
    }
}

fn process(pid: i32) -> Process {
    Process {
        key: ProcessKey {
            pid,
            boot_id: "boot".into(),
            start_time_ticks: 0,
        },
        pid,
        start_time_ticks: 0,
        boot_id: "boot".into(),
        user: Some("u".into()),
        exe: None,
        cmdline: vec![],
        cwd: None,
        ppid: None,
        pgid: None,
        sid: None,
        cgroup: None,
        netns: Some("host".into()),
        container_id: None,
        systemd_unit: None,
        lazyadmin_run_id: None,
        environment: RedactedEnvironmentSummary { keys: vec![] },
        provenance: vec![],
    }
}

fn workload(id: &str) -> Workload {
    Workload {
        id: WorkloadId::new(id),
        display_name: id.into(),
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
    }
}

fn manager(id: &str) -> Manager {
    Manager {
        id: ManagerId::new(id),
        kind: RuntimeKind::Docker,
        name: id.into(),
        scope: ManagerScope::System,
        socket: None,
        available: true,
        permission: PermissionState::Ok,
        version: None,
        provenance: vec![],
    }
}

#[test]
fn merging_no_outputs_yields_default_empty_graph() {
    let g = Graph::merge_outputs(vec![]);
    assert!(g.listeners.is_empty());
    assert!(g.processes.is_empty());
    assert!(g.workloads.is_empty());
    assert!(g.managers.is_empty());
    assert!(g.projects.is_empty());
    assert!(g.tracked_runs.is_empty());
    assert!(g.edges.is_empty());
    assert!(g.warnings.is_empty());
}

#[test]
fn later_output_overrides_earlier_listener_with_same_id() {
    let out1 = DiscoveryOutput {
        listeners: vec![listener("dup", Confidence::Low, vec![])],
        ..Default::default()
    };
    let out2 = DiscoveryOutput {
        listeners: vec![listener("dup", Confidence::High, vec![])],
        ..Default::default()
    };
    let g = Graph::merge_outputs(vec![out1, out2]);
    assert_eq!(g.listeners.len(), 1);
    assert_eq!(
        g.listeners[&ListenerId::new("dup")].confidence,
        Confidence::High
    );
}

#[test]
fn merging_lifts_listener_confidence_to_max_of_its_provenance() {
    // listener has Low base confidence but a Medium and a High provenance.
    // merge_outputs should bump confidence to High.
    let l = listener(
        "x",
        Confidence::Low,
        vec![prov("p1", Confidence::Medium), prov("p2", Confidence::High)],
    );
    let g = Graph::merge_outputs(vec![DiscoveryOutput {
        listeners: vec![l],
        ..Default::default()
    }]);
    assert_eq!(
        g.listeners[&ListenerId::new("x")].confidence,
        Confidence::High
    );
}

#[test]
fn confidence_stays_at_input_value_when_no_provenance() {
    let l = listener("x", Confidence::Medium, vec![]);
    let g = Graph::merge_outputs(vec![DiscoveryOutput {
        listeners: vec![l],
        ..Default::default()
    }]);
    assert_eq!(
        g.listeners[&ListenerId::new("x")].confidence,
        Confidence::Medium
    );
}

#[test]
fn merging_concatenates_edges_and_warnings() {
    let edge = Edge {
        kind: EdgeKind::WorkloadOwnsListener,
        from: EntityRef::Workload(WorkloadId::new("w")),
        to: EntityRef::Listener(ListenerId::new("l")),
        provenance: vec![],
    };
    let warning = Warning {
        severity: WarningSeverity::Warning,
        code: "PUBLIC".into(),
        message: "m".into(),
        entity: None,
        provenance: vec![],
    };
    let out1 = DiscoveryOutput {
        edges: vec![edge.clone()],
        warnings: vec![warning.clone()],
        ..Default::default()
    };
    let out2 = DiscoveryOutput {
        edges: vec![edge.clone()],
        warnings: vec![warning.clone()],
        ..Default::default()
    };
    let g = Graph::merge_outputs(vec![out1, out2]);
    assert_eq!(g.edges.len(), 2, "edges concatenate, not de-dupe");
    assert_eq!(g.warnings.len(), 2);
}

#[test]
fn merging_unique_processes_workloads_managers_into_indexmaps() {
    let out = DiscoveryOutput {
        processes: vec![process(10), process(11)],
        workloads: vec![workload("a"), workload("b")],
        managers: vec![manager("m")],
        ..Default::default()
    };
    let g = Graph::merge_outputs(vec![out]);
    assert_eq!(g.processes.len(), 2);
    assert_eq!(g.workloads.len(), 2);
    assert_eq!(g.managers.len(), 1);
}

#[test]
fn graph_default_is_empty() {
    let g = Graph::default();
    assert_eq!(g.listeners.len(), 0);
    assert_eq!(g.edges.len(), 0);
}

#[test]
fn discovery_output_default_is_empty() {
    let out = DiscoveryOutput::default();
    assert!(out.listeners.is_empty());
    assert!(out.processes.is_empty());
    assert!(out.workloads.is_empty());
    assert!(out.managers.is_empty());
    assert!(out.projects.is_empty());
    assert!(out.tracked_runs.is_empty());
    assert!(out.edges.is_empty());
    assert!(out.warnings.is_empty());
}

#[test]
fn discovery_output_round_trips_through_json() {
    let out = DiscoveryOutput {
        listeners: vec![listener("l", Confidence::High, vec![])],
        workloads: vec![workload("w")],
        ..Default::default()
    };
    let json = serde_json::to_string(&out).unwrap();
    let back: DiscoveryOutput = serde_json::from_str(&json).unwrap();
    assert_eq!(back.listeners.len(), 1);
    assert_eq!(back.workloads.len(), 1);
}

#[test]
fn adapter_capabilities_default_is_neither_poll_nor_watch() {
    let caps = AdapterCapabilities::default();
    assert!(!caps.polling);
    assert!(!caps.watching);
}
