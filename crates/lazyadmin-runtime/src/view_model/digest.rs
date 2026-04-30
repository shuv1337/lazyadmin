use lazyadmin_core::model::{Exposure, Snapshot, WarningSeverity};

use super::doctor_groups::{WarningGroup, build_doctor_groups};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Digest {
    pub exposed: usize,
    pub conflicts: usize,
    pub your_projects: usize,
    pub triage: Vec<WarningGroup>,
}

impl Digest {
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        Self {
            exposed: snapshot
                .listeners
                .iter()
                .filter(|listener| {
                    matches!(listener.exposure, Exposure::LanOrPublic | Exposure::Public)
                })
                .count(),
            conflicts: snapshot
                .warnings
                .iter()
                .filter(|warning| {
                    warning.code == "CONFLICT" || warning.severity == WarningSeverity::Error
                })
                .count(),
            your_projects: snapshot
                .projects
                .iter()
                .filter(|project| {
                    snapshot.workloads.iter().any(|workload| {
                        workload.project.as_ref() == Some(&project.id)
                            && !workload.listeners.is_empty()
                    })
                })
                .count(),
            triage: build_doctor_groups(snapshot)
                .groups
                .into_iter()
                .take(3)
                .collect(),
        }
    }
}
