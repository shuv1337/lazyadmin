use crate::model::*;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Diff {
    pub schema_version: String,
    pub generated_at: chrono::DateTime<chrono::Utc>,
    pub listeners: EntityChanges<ListenerId>,
    pub workloads: EntityChanges<WorkloadId>,
    pub owner_changes: Vec<OwnerChange>,
    pub warning_changes: EntityChanges<String>,
    pub summaries: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntityChanges<T> {
    pub added: Vec<T>,
    pub removed: Vec<T>,
    pub changed: Vec<T>,
}
impl<T> Default for EntityChanges<T> {
    fn default() -> Self {
        Self {
            added: vec![],
            removed: vec![],
            changed: vec![],
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct OwnerChange {
    pub listener: ListenerId,
    pub before: Vec<EntityRef>,
    pub after: Vec<EntityRef>,
}

pub fn diff_snapshots(before: &Snapshot, after: &Snapshot) -> Diff {
    let listeners = changes_by(before.listeners.iter(), after.listeners.iter(), |l| {
        l.id.clone()
    });
    let workloads = changes_by(before.workloads.iter(), after.workloads.iter(), |w| {
        w.id.clone()
    });
    let mut owner_changes = vec![];
    for bl in &before.listeners {
        if let Some(al) = after.listeners.iter().find(|x| x.id == bl.id) {
            if bl.owners != al.owners {
                owner_changes.push(OwnerChange {
                    listener: bl.id.clone(),
                    before: bl.owners.clone(),
                    after: al.owners.clone(),
                });
            }
        }
    }
    let warning_changes = changes_by(before.warnings.iter(), after.warnings.iter(), |w| {
        w.code.clone()
    });
    let summaries = vec![
        format!(
            "listeners: +{} -{} ~{}",
            listeners.added.len(),
            listeners.removed.len(),
            listeners.changed.len()
        ),
        format!(
            "workloads: +{} -{} ~{}",
            workloads.added.len(),
            workloads.removed.len(),
            workloads.changed.len()
        ),
    ];
    Diff {
        schema_version: DIFF_SCHEMA_VERSION.into(),
        generated_at: chrono::Utc::now(),
        listeners,
        workloads,
        owner_changes,
        warning_changes,
        summaries,
    }
}

fn changes_by<T, K, F>(
    before: impl Iterator<Item = T>,
    after: impl Iterator<Item = T>,
    key: F,
) -> EntityChanges<K>
where
    T: Clone + PartialEq,
    K: Clone + Eq + std::hash::Hash,
    F: Fn(&T) -> K,
{
    use std::collections::HashMap;
    let b: HashMap<K, T> = before.map(|v| (key(&v), v)).collect();
    let a: HashMap<K, T> = after.map(|v| (key(&v), v)).collect();
    let mut out = EntityChanges::default();
    for (k, v) in &a {
        match b.get(k) {
            None => out.added.push(k.clone()),
            Some(old) if old != v => out.changed.push(k.clone()),
            _ => {}
        }
    }
    for k in b.keys() {
        if !a.contains_key(k) {
            out.removed.push(k.clone());
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn listener(id: &str, owners: Vec<EntityRef>) -> Listener {
        // Use a fixed timestamp so two listeners constructed with identical
        // inputs compare equal.
        let ts = chrono::DateTime::parse_from_rfc3339("2026-01-01T00:00:00Z")
            .unwrap()
            .to_utc();
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
            owners,
            confidence: Confidence::High,
            provenance: vec![],
            first_seen: ts,
            last_seen: ts,
            dual_stack_state: DualStackState::NotApplicable,
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

    fn warning(code: &str) -> Warning {
        Warning {
            severity: WarningSeverity::Warning,
            code: code.into(),
            message: "m".into(),
            entity: None,
            provenance: vec![],
        }
    }

    #[test]
    fn empty_diff_has_zero_changes() {
        let d = diff_snapshots(&Snapshot::empty(), &Snapshot::empty());
        assert_eq!(d.schema_version, DIFF_SCHEMA_VERSION);
        assert!(d.listeners.added.is_empty());
        insta::assert_json_snapshot!(d.summaries);
    }

    #[test]
    fn detects_added_listener() {
        let before = Snapshot::empty();
        let mut after = Snapshot::empty();
        after.listeners.push(listener("new", vec![]));
        let d = diff_snapshots(&before, &after);
        assert_eq!(d.listeners.added.len(), 1);
        assert_eq!(d.listeners.added[0], ListenerId::new("new"));
        assert!(d.listeners.removed.is_empty());
        assert!(d.listeners.changed.is_empty());
    }

    #[test]
    fn detects_removed_listener() {
        let mut before = Snapshot::empty();
        before.listeners.push(listener("gone", vec![]));
        let after = Snapshot::empty();
        let d = diff_snapshots(&before, &after);
        assert_eq!(d.listeners.removed.len(), 1);
        assert_eq!(d.listeners.removed[0], ListenerId::new("gone"));
    }

    #[test]
    fn detects_changed_listener_when_owners_differ() {
        let mut before = Snapshot::empty();
        before.listeners.push(listener("l", vec![]));
        let mut after = Snapshot::empty();
        after.listeners.push(listener(
            "l",
            vec![EntityRef::Workload(WorkloadId::new("w"))],
        ));
        let d = diff_snapshots(&before, &after);
        assert!(d.listeners.added.is_empty());
        assert!(d.listeners.removed.is_empty());
        assert_eq!(d.listeners.changed, vec![ListenerId::new("l")]);
        assert_eq!(d.owner_changes.len(), 1);
        assert_eq!(d.owner_changes[0].listener, ListenerId::new("l"));
        assert!(d.owner_changes[0].before.is_empty());
        assert_eq!(d.owner_changes[0].after.len(), 1);
    }

    #[test]
    fn does_not_emit_owner_change_when_owners_identical() {
        let mut before = Snapshot::empty();
        before.listeners.push(listener(
            "l",
            vec![EntityRef::Workload(WorkloadId::new("w"))],
        ));
        let mut after = Snapshot::empty();
        after.listeners.push(listener(
            "l",
            vec![EntityRef::Workload(WorkloadId::new("w"))],
        ));
        let d = diff_snapshots(&before, &after);
        assert!(d.owner_changes.is_empty());
        assert!(d.listeners.changed.is_empty());
    }

    #[test]
    fn workload_added_removed_and_changed() {
        let mut before = Snapshot::empty();
        before.workloads.push(workload("a"));
        before.workloads.push(workload("keep"));
        let mut after = Snapshot::empty();
        after.workloads.push(workload("keep"));
        let mut changed_keep = workload("keep");
        // mutate to force "changed" but keep same id
        changed_keep.display_name = "renamed".into();
        after.workloads[0] = changed_keep;
        after.workloads.push(workload("new"));
        let d = diff_snapshots(&before, &after);
        assert!(d.workloads.added.contains(&WorkloadId::new("new")));
        assert!(d.workloads.removed.contains(&WorkloadId::new("a")));
        assert!(d.workloads.changed.contains(&WorkloadId::new("keep")));
    }

    #[test]
    fn warning_changes_keyed_by_code() {
        let mut before = Snapshot::empty();
        before.warnings.push(warning("OLD"));
        before.warnings.push(warning("SAME"));
        let mut after = Snapshot::empty();
        after.warnings.push(warning("SAME"));
        after.warnings.push(warning("NEW"));
        let d = diff_snapshots(&before, &after);
        assert!(d.warning_changes.added.iter().any(|s| s == "NEW"));
        assert!(d.warning_changes.removed.iter().any(|s| s == "OLD"));
    }

    #[test]
    fn summaries_report_counts_for_listeners_and_workloads() {
        let before = Snapshot::empty();
        let mut after = Snapshot::empty();
        after.listeners.push(listener("l1", vec![]));
        after.workloads.push(workload("w1"));
        let d = diff_snapshots(&before, &after);
        // Summaries are deterministic strings.
        assert!(d.summaries.iter().any(|s| s.contains("listeners")));
        assert!(d.summaries.iter().any(|s| s.contains("workloads")));
        assert!(d.summaries[0].contains("+1"));
    }

    #[test]
    fn entity_changes_default_is_all_empty() {
        let c: EntityChanges<String> = EntityChanges::default();
        assert!(c.added.is_empty());
        assert!(c.removed.is_empty());
        assert!(c.changed.is_empty());
    }

    #[test]
    fn diff_round_trips_through_json() {
        let d = diff_snapshots(&Snapshot::empty(), &Snapshot::empty());
        let json = serde_json::to_string(&d).unwrap();
        let back: Diff = serde_json::from_str(&json).unwrap();
        assert_eq!(back.schema_version, d.schema_version);
        assert_eq!(back.summaries, d.summaries);
    }
}
