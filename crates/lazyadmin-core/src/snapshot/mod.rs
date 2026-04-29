use crate::{
    config::Config,
    correlate::{EventDropCounter, correlate},
    graph::{DiscoveryOutput, Graph},
    model::*,
};

#[derive(Clone, Debug, Default)]
pub struct SnapshotBuilder;

impl SnapshotBuilder {
    #[tracing::instrument(name = "snapshot.build", skip_all, fields(adapter_count = outputs.len()))]
    pub fn from_adapter_outputs(outputs: Vec<DiscoveryOutput>) -> Snapshot {
        Self::from_adapter_outputs_with_config(outputs, &Config::default())
    }

    #[tracing::instrument(name = "snapshot.build", skip_all, fields(adapter_count = outputs.len()))]
    pub fn from_adapter_outputs_with_config(
        outputs: Vec<DiscoveryOutput>,
        cfg: &Config,
    ) -> Snapshot {
        let graph = Graph::merge_outputs(outputs);
        let graph = correlate(graph, cfg);
        Self::from_graph(graph)
    }

    #[tracing::instrument(name = "snapshot.build", skip_all, fields(adapter_count = outputs.len(), events_dropped = drops.dropped()))]
    pub fn from_adapter_outputs_with_event_drops(
        outputs: Vec<DiscoveryOutput>,
        drops: &EventDropCounter,
    ) -> Snapshot {
        Self::from_adapter_outputs_with_config_and_event_drops(outputs, &Config::default(), drops)
    }

    #[tracing::instrument(name = "snapshot.build", skip_all, fields(adapter_count = outputs.len(), events_dropped = drops.dropped()))]
    pub fn from_adapter_outputs_with_config_and_event_drops(
        outputs: Vec<DiscoveryOutput>,
        cfg: &Config,
        drops: &EventDropCounter,
    ) -> Snapshot {
        let graph = Graph::merge_outputs(outputs);
        let graph = correlate(graph, cfg);
        Self::from_graph_with_event_drops(graph, drops.dropped())
    }

    #[tracing::instrument(name = "graph.correlate", skip_all, fields(result = "ok"))]
    pub fn from_graph(graph: Graph) -> Snapshot {
        Self::from_graph_with_event_drop_count(graph, 0)
    }

    pub fn from_graph_with_event_drops(graph: Graph, drops: u64) -> Snapshot {
        Self::from_graph_with_event_drop_count(graph, drops)
    }

    fn from_graph_with_event_drop_count(mut graph: Graph, events_dropped: u64) -> Snapshot {
        if events_dropped > 0 {
            graph.warnings.push(Warning {
                severity: WarningSeverity::Warning,
                code: "EVENTS_DROPPED".into(),
                message: format!(
                    "{events_dropped} discovery event(s) were dropped by the bounded fan-in; snapshot may lag until the next full scan"
                ),
                entity: None,
                provenance: vec![],
            });
        }
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
            metadata: (events_dropped > 0).then_some(SnapshotMetadata {
                events_dropped: Some(events_dropped),
            }),
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

    #[test]
    fn nonzero_event_drops_are_snapshotted() {
        let snap = SnapshotBuilder::from_graph_with_event_drops(Graph::default(), 3);
        assert_eq!(snap.metadata.unwrap().events_dropped, Some(3));
        assert!(snap.warnings.iter().any(|w| w.code == "EVENTS_DROPPED"));
    }

    #[test]
    fn portless_snapshot_fixture_roundtrips() {
        let text = include_str!("../../../../testdata/snapshots/portless.json");
        let snap: Snapshot = serde_json::from_str(text).unwrap();
        assert!(snap.workloads.iter().any(|workload| {
            workload.runtime == RuntimeKind::Portless && workload.display_name == "demo"
        }));
        assert!(snap.workloads.iter().any(|workload| {
            workload.runtime == RuntimeKind::Portless && workload.display_name == "alias"
        }));
        serde_json::to_string(&snap).unwrap();
    }
}
