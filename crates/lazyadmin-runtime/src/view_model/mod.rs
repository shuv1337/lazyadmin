use lazyadmin_core::model::Snapshot;
use serde::{Deserialize, Serialize};

pub mod digest;
pub mod doctor_groups;
pub mod header_pip;
pub mod inspector;
pub mod search;

pub use digest::{
    ConflictsSection, Digest, DigestViewTarget, ExposedRow, ExposedSection, ProjectRow,
    ProjectsSection, TriageSection, build_digest,
};
pub use doctor_groups::{
    DoctorGroupsView, TriageSummary, WarningGroup, build_doctor_groups, warning_groups,
};
pub use header_pip::{AdapterHealth, DropRate, HeaderPip, SnapshotFreshness};
pub use inspector::InspectorView;
pub use search::{
    DEFAULT_SEARCH_LIMIT, ListenerHit, MAX_SEARCH_LIMIT, ManagerHit, ProcessHit, ProjectHit,
    RailViewHit, SEARCH_SCHEMA_VERSION, SearchGroup, SearchHitRef, SearchKind, SearchKinds,
    SearchOptions, SearchQuery, SearchResults, WorkloadHit, run, search_hit_at, search_hit_count,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct RailEntry {
    pub id: &'static str,
    pub label: &'static str,
}

pub const RAIL_ENTRIES: &[RailEntry] = &[
    RailEntry {
        id: "overview",
        label: "Overview",
    },
    RailEntry {
        id: "listeners",
        label: "Listeners",
    },
    RailEntry {
        id: "workloads",
        label: "Workloads",
    },
    RailEntry {
        id: "processes",
        label: "Processes",
    },
    RailEntry {
        id: "doctor",
        label: "Doctor",
    },
    RailEntry {
        id: "metrics",
        label: "Metrics",
    },
];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RuntimeViewModels {
    pub digest: Digest,
    pub inspector: Vec<InspectorView>,
    pub doctor_groups: DoctorGroupsView,
    pub header_pip: HeaderPip,
}

impl RuntimeViewModels {
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        Self {
            digest: Digest::from_snapshot(snapshot),
            inspector: InspectorView::all_from_snapshot(snapshot),
            doctor_groups: build_doctor_groups(snapshot),
            header_pip: HeaderPip::from_snapshot(snapshot),
        }
    }
}

#[cfg(test)]
mod tests;
