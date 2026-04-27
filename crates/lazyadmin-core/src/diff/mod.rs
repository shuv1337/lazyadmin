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
    #[test]
    fn empty_diff_has_zero_changes() {
        let d = diff_snapshots(&Snapshot::empty(), &Snapshot::empty());
        assert_eq!(d.schema_version, DIFF_SCHEMA_VERSION);
        assert!(d.listeners.added.is_empty());
        insta::assert_json_snapshot!(d.summaries);
    }
}
