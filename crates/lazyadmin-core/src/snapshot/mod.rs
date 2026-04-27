use crate::{
    graph::{DiscoveryOutput, Graph},
    model::*,
};

#[derive(Clone, Debug, Default)]
pub struct SnapshotBuilder;

impl SnapshotBuilder {
    #[tracing::instrument(name = "snapshot.build", skip_all, fields(adapter_count = outputs.len()))]
    pub fn from_adapter_outputs(outputs: Vec<DiscoveryOutput>) -> Snapshot {
        let graph = Graph::merge_outputs(outputs);
        Self::from_graph(graph)
    }
    #[tracing::instrument(name = "graph.correlate", skip_all, fields(result = "ok"))]
    pub fn from_graph(graph: Graph) -> Snapshot {
        Snapshot {
            schema_version: SNAPSHOT_SCHEMA_VERSION.into(),
            generated_at: chrono::Utc::now(),
            host: Host {
                boot_id: None,
                hostname: None,
                kernel: None,
            },
            managers: graph.managers.into_values().collect(),
            processes: graph.processes.into_values().collect(),
            listeners: graph.listeners.into_values().collect(),
            workloads: graph.workloads.into_values().collect(),
            projects: graph.projects.into_values().collect(),
            tracked_runs: graph.tracked_runs.into_values().collect(),
            edges: graph.edges,
            warnings: graph.warnings,
        }
    }
    pub fn empty() -> Snapshot {
        Self::from_adapter_outputs(vec![])
    }
}

pub fn build_empty_snapshot() -> Snapshot {
    SnapshotBuilder::empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn empty_snapshot_roundtrip() {
        let snap = SnapshotBuilder::empty();
        let json = serde_json::to_string(&snap).unwrap();
        let back: Snapshot = serde_json::from_str(&json).unwrap();
        assert_eq!(back.schema_version, SNAPSHOT_SCHEMA_VERSION);
        assert!(back.listeners.is_empty());
    }
    #[test]
    fn empty_snapshot_golden_shape() {
        let mut snap = SnapshotBuilder::empty();
        snap.generated_at = chrono::DateTime::parse_from_rfc3339("2026-04-27T12:00:00Z")
            .unwrap()
            .to_utc();
        insta::assert_json_snapshot!(snap);
    }
}
