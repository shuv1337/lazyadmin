use lazyadmin_core::model::{
    EntityRef, Listener, Manager, ManagerId, ManagerScope, Process, ProcessKey, Project, ProjectId,
    Snapshot, Workload, WorkloadId,
};

#[derive(Clone, Copy, Debug)]
pub struct SnapshotRelations<'a> {
    snapshot: &'a Snapshot,
}

impl<'a> SnapshotRelations<'a> {
    pub fn new(snapshot: &'a Snapshot) -> Self {
        Self { snapshot }
    }

    pub fn process(&self, key: &ProcessKey) -> Option<&'a Process> {
        self.snapshot
            .processes
            .iter()
            .find(|process| &process.key == key)
    }

    pub fn workload(&self, id: &WorkloadId) -> Option<&'a Workload> {
        self.snapshot
            .workloads
            .iter()
            .find(|workload| &workload.id == id)
    }

    pub fn manager(&self, id: &ManagerId) -> Option<&'a Manager> {
        self.snapshot
            .managers
            .iter()
            .find(|manager| &manager.id == id)
    }

    pub fn project(&self, id: &ProjectId) -> Option<&'a Project> {
        self.snapshot
            .projects
            .iter()
            .find(|project| &project.id == id)
    }

    pub fn listener_bind_label(&self, listener: &Listener) -> String {
        if let Some(path) = &listener.path {
            return path.display().to_string();
        }
        let addr = listener.bind_addr.as_deref().unwrap_or("*");
        match listener.port {
            Some(port) => format!("{addr}:{port}"),
            None => addr.to_string(),
        }
    }

    pub fn listener_owner_label(&self, listener: &Listener, unowned_label: &str) -> String {
        match listener.owners.first() {
            Some(owner) => self.entity_display_label(owner),
            None => unowned_label.to_string(),
        }
    }

    pub fn listener_search_owner_label(&self, listener: &Listener) -> String {
        listener
            .owners
            .first()
            .map(|owner| self.entity_search_label(owner))
            .unwrap_or_default()
    }

    pub fn entity_display_label(&self, entity: &EntityRef) -> String {
        match entity {
            EntityRef::Process(key) => format!("pid {}", key.pid),
            EntityRef::Workload(id) => self
                .workload(id)
                .map(|workload| workload.display_name.clone())
                .unwrap_or_else(|| format!("workload {id}")),
            EntityRef::Manager(id) => self
                .manager(id)
                .map(|manager| manager.name.clone())
                .unwrap_or_else(|| format!("manager {id}")),
            EntityRef::Project(id) => self
                .project(id)
                .map(|project| project.name.clone())
                .unwrap_or_else(|| format!("project {id}")),
            EntityRef::Run(id) => format!("tracked run {id}"),
            EntityRef::Listener(id) => id.to_string(),
            EntityRef::Action(id) => id.to_string(),
        }
    }

    pub fn entity_search_label(&self, entity: &EntityRef) -> String {
        match entity {
            EntityRef::Process(key) => self
                .process(key)
                .and_then(|process| process.exe.as_ref().map(|exe| exe.display().to_string()))
                .unwrap_or_else(|| key.pid.to_string()),
            EntityRef::Workload(id) => self
                .workload(id)
                .map(|workload| workload.display_name.clone())
                .unwrap_or_else(|| id.to_string()),
            EntityRef::Manager(id) => self
                .manager(id)
                .map(|manager| manager.name.clone())
                .unwrap_or_else(|| id.to_string()),
            EntityRef::Project(id) => self
                .project(id)
                .map(|project| project.name.clone())
                .unwrap_or_else(|| id.to_string()),
            EntityRef::Listener(id) => id.to_string(),
            EntityRef::Run(id) => id.to_string(),
            EntityRef::Action(id) => id.to_string(),
        }
    }

    pub fn listener_owner_pid(&self, listener: &Listener) -> Option<i32> {
        for owner in &listener.owners {
            match owner {
                EntityRef::Process(key) => return Some(key.pid),
                EntityRef::Workload(id) => {
                    if let Some(pid) = self
                        .workload(id)
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

    pub fn listener_project_label(&self, listener: &Listener) -> Option<String> {
        self.listener_project(listener)
            .map(|project| project.name.clone())
            .or_else(|| self.listener_project_id(listener).map(ToString::to_string))
    }

    pub fn listener_project(&self, listener: &Listener) -> Option<&'a Project> {
        self.listener_project_id(listener)
            .and_then(|project_id| self.project(project_id))
    }

    pub fn listener_attached_workload_labels(&self, listener: &Listener) -> Vec<String> {
        self.snapshot
            .workloads
            .iter()
            .filter(|workload| workload.listeners.iter().any(|id| id == &listener.id))
            .map(|workload| workload.display_name.clone())
            .collect()
    }

    pub fn listener_attached_project_label(&self, listener: &Listener) -> Option<String> {
        self.snapshot
            .workloads
            .iter()
            .filter(|workload| workload.listeners.iter().any(|id| id == &listener.id))
            .filter_map(|workload| workload.project.as_ref())
            .find_map(|project_id| self.project(project_id).map(|project| project.name.clone()))
    }

    pub fn workload_project_label(&self, workload: &Workload) -> Option<String> {
        workload
            .project
            .as_ref()
            .and_then(|project_id| self.project(project_id))
            .map(|project| project.name.clone())
    }

    pub fn workload_manager_label(&self, workload: &Workload) -> Option<String> {
        workload
            .manager
            .as_ref()
            .and_then(|manager_id| self.manager(manager_id))
            .map(|manager| manager.name.clone())
    }

    pub fn is_system_listener(&self, listener: &Listener) -> bool {
        for owner in &listener.owners {
            match owner {
                EntityRef::Manager(id)
                    if self
                        .manager(id)
                        .is_some_and(|manager| manager.scope == ManagerScope::System) =>
                {
                    return true;
                }
                EntityRef::Process(key)
                    if self.process(key).is_some_and(|process| {
                        process.user.as_deref() == Some("root")
                            && process
                                .systemd_unit
                                .as_deref()
                                .map(|unit| !unit.contains("user"))
                                .unwrap_or(true)
                    }) =>
                {
                    return true;
                }
                _ => {}
            }
        }
        false
    }

    fn listener_project_id(&self, listener: &Listener) -> Option<&'a ProjectId> {
        for owner in &listener.owners {
            let project_id = match owner {
                EntityRef::Workload(id) => self
                    .workload(id)
                    .and_then(|workload| workload.project.as_ref()),
                EntityRef::Process(key) => self
                    .snapshot
                    .workloads
                    .iter()
                    .find(|workload| workload.pids.iter().any(|pid| pid == key))
                    .and_then(|workload| workload.project.as_ref()),
                _ => None,
            };
            if project_id.is_some() {
                return project_id;
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;
    use lazyadmin_core::model::{
        AddressFamily, Confidence, DualStackState, EntityRef, Exposure, ListenerId, ListenerState,
        PermissionState, Protocol, RedactedEnvironmentSummary, RuntimeKind,
    };

    fn empty_snapshot() -> Snapshot {
        serde_json::from_str(include_str!("../../../../testdata/snapshots/empty.json")).unwrap()
    }

    fn busy_snapshot() -> Snapshot {
        serde_json::from_str(include_str!("../../../../testdata/snapshots/busy.json")).unwrap()
    }

    #[test]
    fn empty_snapshot_relations_return_absent_values() {
        let snapshot = empty_snapshot();
        let relations = SnapshotRelations::new(&snapshot);
        let listener = Listener {
            id: ListenerId::new("tcp:*:65535"),
            protocol: Protocol::Tcp,
            family: AddressFamily::Ipv4,
            bind_addr: None,
            port: Some(65535),
            path: None,
            state: ListenerState::Listen,
            netns: "host".into(),
            socket_inode: None,
            exposure: Exposure::Loopback,
            owners: Vec::new(),
            confidence: Confidence::Low,
            provenance: Vec::new(),
            first_seen: Utc::now(),
            last_seen: Utc::now(),
            dual_stack_state: DualStackState::Unknown,
        };

        assert_eq!(relations.listener_bind_label(&listener), "*:65535");
        assert_eq!(
            relations.listener_owner_label(&listener, "unowned"),
            "unowned"
        );
        assert_eq!(relations.listener_owner_pid(&listener), None);
        assert_eq!(relations.listener_project_label(&listener), None);
        assert!(!relations.is_system_listener(&listener));
    }

    #[test]
    fn busy_snapshot_listener_facts_match_known_fixture() {
        let snapshot = busy_snapshot();
        let relations = SnapshotRelations::new(&snapshot);
        let listener = snapshot
            .listeners
            .iter()
            .find(|listener| listener.id == ListenerId::new("tcp:127.0.0.1:8080"))
            .expect("fixture listener");

        assert_eq!(relations.listener_bind_label(listener), "127.0.0.1:8080");
        assert_eq!(
            relations.listener_owner_label(listener, "—"),
            "api dev server"
        );
        assert_eq!(relations.listener_owner_pid(listener), Some(17385));
        assert_eq!(
            relations.listener_project_label(listener),
            Some("api".into())
        );
        assert_eq!(
            relations.listener_attached_workload_labels(listener),
            vec!["api dev server"]
        );
        assert!(!relations.is_system_listener(listener));
    }

    #[test]
    fn listener_bind_label_prefers_unix_path_and_star_addr_fallback() {
        let snapshot = Snapshot::empty();
        let relations = SnapshotRelations::new(&snapshot);
        let mut listener = Listener {
            id: ListenerId::new("unix:/tmp/app.sock"),
            protocol: Protocol::Unix,
            family: AddressFamily::Unix,
            bind_addr: None,
            port: None,
            path: Some("/tmp/app.sock".into()),
            state: ListenerState::Listen,
            netns: "host".into(),
            socket_inode: None,
            exposure: Exposure::UnixLocal,
            owners: Vec::new(),
            confidence: Confidence::High,
            provenance: Vec::new(),
            first_seen: Utc::now(),
            last_seen: Utc::now(),
            dual_stack_state: DualStackState::Unknown,
        };
        assert_eq!(relations.listener_bind_label(&listener), "/tmp/app.sock");
        listener.path = None;
        listener.port = Some(9000);
        assert_eq!(relations.listener_bind_label(&listener), "*:9000");
    }

    #[test]
    fn system_listener_classifies_manager_and_root_process_owners() {
        let mut snapshot = Snapshot::empty();
        snapshot.managers.push(Manager {
            id: ManagerId::new("manager:systemd-system"),
            name: "systemd system".into(),
            kind: RuntimeKind::SystemdSystem,
            scope: ManagerScope::System,
            available: true,
            permission: PermissionState::Ok,
            version: None,
            socket: None,
            provenance: Vec::new(),
        });
        let manager_listener = Listener {
            id: ListenerId::new("tcp:0.0.0.0:22"),
            protocol: Protocol::Tcp,
            family: AddressFamily::Ipv4,
            bind_addr: Some("0.0.0.0".into()),
            port: Some(22),
            path: None,
            state: ListenerState::Listen,
            netns: "host".into(),
            socket_inode: None,
            exposure: Exposure::Public,
            owners: vec![EntityRef::Manager(ManagerId::new("manager:systemd-system"))],
            confidence: Confidence::High,
            provenance: Vec::new(),
            first_seen: Utc::now(),
            last_seen: Utc::now(),
            dual_stack_state: DualStackState::Unknown,
        };
        let root_key = ProcessKey {
            pid: 1,
            boot_id: "boot".into(),
            start_time_ticks: 1,
        };
        snapshot.processes.push(Process {
            key: root_key.clone(),
            pid: 1,
            start_time_ticks: 1,
            boot_id: "boot".into(),
            user: Some("root".into()),
            exe: Some("/sbin/init".into()),
            cmdline: vec!["/sbin/init".into()],
            cwd: None,
            ppid: None,
            pgid: None,
            sid: None,
            cgroup: None,
            netns: None,
            container_id: None,
            systemd_unit: Some("init.scope".into()),
            lazyadmin_run_id: None,
            environment: RedactedEnvironmentSummary::default(),
            provenance: Vec::new(),
        });
        let root_process_listener = Listener {
            owners: vec![EntityRef::Process(root_key)],
            id: ListenerId::new("tcp:0.0.0.0:25"),
            port: Some(25),
            ..manager_listener.clone()
        };
        let relations = SnapshotRelations::new(&snapshot);
        assert!(relations.is_system_listener(&manager_listener));
        assert!(relations.is_system_listener(&root_process_listener));
    }
}
