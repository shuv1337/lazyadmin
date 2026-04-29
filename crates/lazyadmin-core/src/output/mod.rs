use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct MessageOutput {
    pub message: String,
}

use crate::model::{
    EdgeKind, EntityRef, Listener, ListenerId, ProcessKey, Protocol, RuntimeKind, Snapshot,
};

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct ListenerRow {
    pub id: ListenerId,
    pub protocol: Protocol,
    pub bind_addr: Option<String>,
    pub port: Option<u16>,
    pub owners_count: usize,
    pub manager_label: Option<String>,
    pub manager_detail: Option<String>,
}

pub fn listener_rows(snapshot: &Snapshot) -> Vec<ListenerRow> {
    snapshot
        .listeners
        .iter()
        .map(|listener| {
            let (manager_label, manager_detail) = manager_projection(snapshot, listener);
            ListenerRow {
                id: listener.id.clone(),
                protocol: listener.protocol.clone(),
                bind_addr: listener.bind_addr.clone(),
                port: listener.port,
                owners_count: listener.owners.len(),
                manager_label,
                manager_detail,
            }
        })
        .collect()
}

fn manager_projection(
    snapshot: &Snapshot,
    listener: &Listener,
) -> (Option<String>, Option<String>) {
    let listener_ref = EntityRef::Listener(listener.id.clone());
    for edge in snapshot
        .edges
        .iter()
        .filter(|edge| edge.kind == EdgeKind::WorkloadOwnsListener && edge.to == listener_ref)
    {
        let EntityRef::Workload(workload_id) = &edge.from else {
            continue;
        };
        let Some(workload) = snapshot
            .workloads
            .iter()
            .find(|workload| &workload.id == workload_id)
        else {
            continue;
        };
        if workload.runtime == RuntimeKind::Portless {
            let cli_pid = match &workload.source {
                Some(EntityRef::Process(ProcessKey { pid, .. })) => Some(*pid),
                _ => None,
            };
            let detail = match cli_pid {
                Some(pid) => format!("portless: {} cli pid {pid}", workload.display_name),
                None => format!("portless: {}", workload.display_name),
            };
            return (Some("portless".into()), Some(detail));
        }
    }
    (None, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    #[test]
    fn listener_row_projects_portless_manager_label() {
        let key = ProcessKey {
            pid: 42,
            boot_id: "boot".into(),
            start_time_ticks: 7,
        };
        let mut snapshot = Snapshot::empty();
        let listener_id = ListenerId::new("tcp:127.0.0.1:3000:1");
        let workload_id = WorkloadId::new("portless:test");
        snapshot.listeners.push(Listener {
            id: listener_id.clone(),
            protocol: Protocol::Tcp,
            family: AddressFamily::Ipv4,
            bind_addr: Some("127.0.0.1".into()),
            port: Some(3000),
            path: None,
            state: ListenerState::Listen,
            netns: "host".into(),
            socket_inode: Some(1),
            exposure: Exposure::Loopback,
            owners: vec![],
            confidence: Confidence::High,
            provenance: vec![],
            first_seen: chrono::Utc::now(),
            last_seen: chrono::Utc::now(),
            dual_stack_state: DualStackState::NotApplicable,
        });
        snapshot.workloads.push(Workload {
            id: workload_id.clone(),
            display_name: "demo.localhost".into(),
            runtime: RuntimeKind::Portless,
            state: WorkloadState::Running,
            pids: vec![],
            listeners: vec![],
            project: None,
            manager: None,
            source: Some(EntityRef::Process(key)),
            actions: vec![],
            health: None,
            metrics: None,
            restart_policy: None,
            lazyadmin_run_id: None,
            provenance: vec![],
        });
        snapshot.edges.push(Edge {
            kind: EdgeKind::WorkloadOwnsListener,
            from: EntityRef::Workload(workload_id),
            to: EntityRef::Listener(listener_id),
            provenance: vec![],
        });

        let row = listener_rows(&snapshot).pop().unwrap();
        assert_eq!(row.manager_label.as_deref(), Some("portless"));
        assert_eq!(
            row.manager_detail.as_deref(),
            Some("portless: demo.localhost cli pid 42")
        );
    }
}
