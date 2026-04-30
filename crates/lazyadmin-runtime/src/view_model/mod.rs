use lazyadmin_core::model::Snapshot;

pub mod digest;
pub mod doctor_groups;
pub mod header_pip;
pub mod inspector;

pub use digest::Digest;
pub use doctor_groups::{
    DoctorGroupsView, TriageSummary, WarningGroup, build_doctor_groups, warning_groups,
};
pub use header_pip::{AdapterHealth, DropRate, HeaderPip, SnapshotFreshness};
pub use inspector::InspectorView;

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
