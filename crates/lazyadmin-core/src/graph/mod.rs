use crate::model::*;
use async_trait::async_trait;
use futures::stream::BoxStream;
use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default)]
pub struct Graph {
    pub listeners: IndexMap<ListenerId, Listener>,
    pub processes: IndexMap<ProcessKey, Process>,
    pub workloads: IndexMap<WorkloadId, Workload>,
    pub managers: IndexMap<ManagerId, Manager>,
    pub projects: IndexMap<ProjectId, Project>,
    pub tracked_runs: IndexMap<RunId, TrackedRun>,
    pub edges: Vec<Edge>,
    pub warnings: Vec<Warning>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct DiscoveryOutput {
    pub managers: Vec<Manager>,
    pub processes: Vec<Process>,
    pub listeners: Vec<Listener>,
    pub workloads: Vec<Workload>,
    pub projects: Vec<Project>,
    pub tracked_runs: Vec<TrackedRun>,
    pub edges: Vec<Edge>,
    pub warnings: Vec<Warning>,
}

#[derive(Clone, Debug, Default)]
pub struct AdapterCapabilities {
    pub polling: bool,
    pub watching: bool,
}
#[derive(Clone, Debug)]
pub struct AdapterHealth {
    pub adapter: String,
    pub available: bool,
    pub message: Option<String>,
}
#[derive(Clone, Debug, Default)]
pub struct DiscoveryContext {}
#[derive(Clone, Debug)]
pub enum Entity {
    Listener(Listener),
    Process(Process),
    Workload(Workload),
    Manager(Manager),
    Project(Project),
    TrackedRun(TrackedRun),
}
#[async_trait]
pub trait DiscoveryAdapter: Send + Sync {
    fn name(&self) -> &'static str;
    fn capabilities(&self) -> AdapterCapabilities;
    async fn health(&self) -> AdapterHealth;
    async fn discover(&self, ctx: DiscoveryContext) -> anyhow::Result<DiscoveryOutput>;
    async fn watch(&self) -> Option<BoxStream<'static, DiscoveryEvent>> {
        None
    }
}

impl Graph {
    #[tracing::instrument(name = "graph.merge", skip_all, fields(entity_counts))]
    pub fn merge_outputs(outputs: Vec<DiscoveryOutput>) -> Self {
        let mut graph = Graph::default();
        for output in outputs {
            for manager in output.managers {
                graph.managers.insert(manager.id.clone(), manager);
            }
            for process in output.processes {
                graph.processes.insert(process.key.clone(), process);
            }
            for mut listener in output.listeners {
                listener.confidence = listener
                    .provenance
                    .iter()
                    .map(|p| p.confidence)
                    .max()
                    .unwrap_or(listener.confidence);
                graph.listeners.insert(listener.id.clone(), listener);
            }
            for workload in output.workloads {
                graph.workloads.insert(workload.id.clone(), workload);
            }
            for project in output.projects {
                graph.projects.insert(project.id.clone(), project);
            }
            for run in output.tracked_runs {
                graph.tracked_runs.insert(run.id.clone(), run);
            }
            graph.edges.extend(output.edges);
            graph.warnings.extend(output.warnings);
        }
        graph
    }
}
