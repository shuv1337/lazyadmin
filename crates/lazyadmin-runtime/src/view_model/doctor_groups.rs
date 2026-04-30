use chrono::{DateTime, Utc};
use lazyadmin_core::{
    doctor::{WarningTier, classify},
    model::{EntityRef, Snapshot, WarningSeverity},
};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct WarningGroup {
    pub code: String,
    pub severity: WarningSeverity,
    pub tier: WarningTier,
    pub label: String,
    pub remediation: String,
    pub count: usize,
    pub sample_entities: Vec<EntityRef>,
    pub expanded: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorGroupsView {
    pub groups: Vec<WarningGroup>,
    pub actionable_count: usize,
    pub noise_group_count: usize,
    pub noise_total_count: usize,
    pub last_check: DateTime<Utc>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TriageSummary {
    pub actionable: usize,
    pub noise_groups: usize,
    pub noise_total: usize,
    pub last_check: DateTime<Utc>,
}

impl DoctorGroupsView {
    pub fn triage_summary(&self) -> TriageSummary {
        TriageSummary {
            actionable: self.actionable_count,
            noise_groups: self.noise_group_count,
            noise_total: self.noise_total_count,
            last_check: self.last_check,
        }
    }
}

pub fn warning_groups(snapshot: &Snapshot) -> Vec<WarningGroup> {
    build_doctor_groups(snapshot).groups
}

pub fn build_doctor_groups(snapshot: &Snapshot) -> DoctorGroupsView {
    let mut groups: Vec<WarningGroup> = Vec::new();
    for warning in &snapshot.warnings {
        let index = groups
            .iter()
            .position(|group| group.code == warning.code && group.severity == warning.severity);
        let entry = if let Some(index) = index {
            &mut groups[index]
        } else {
            let meta = classify(&warning.code);
            groups.push(WarningGroup {
                code: warning.code.clone(),
                severity: warning.severity.clone(),
                tier: meta.tier,
                label: if meta.code == "unknown" {
                    warning.code.clone()
                } else {
                    meta.label.to_string()
                },
                remediation: meta.remediation.to_string(),
                count: 0,
                sample_entities: Vec::new(),
                expanded: !matches!(meta.tier, WarningTier::Noise),
            });
            groups.last_mut().expect("group just pushed")
        };
        entry.count += 1;
        if let Some(entity) = &warning.entity
            && entry.sample_entities.len() < 5
        {
            entry.sample_entities.push(entity.clone());
        }
    }
    groups.sort_by_key(|group| {
        (
            tier_rank(group.tier),
            severity_rank(&group.severity),
            std::cmp::Reverse(group.count),
            group.code.clone(),
        )
    });
    let actionable_count = groups
        .iter()
        .filter(|group| matches!(group.tier, WarningTier::Critical | WarningTier::Actionable))
        .map(|group| group.count)
        .sum();
    let noise_group_count = groups
        .iter()
        .filter(|group| matches!(group.tier, WarningTier::Noise))
        .count();
    let noise_total_count = groups
        .iter()
        .filter(|group| matches!(group.tier, WarningTier::Noise))
        .map(|group| group.count)
        .sum();
    DoctorGroupsView {
        groups,
        actionable_count,
        noise_group_count,
        noise_total_count,
        last_check: snapshot.generated_at,
    }
}

fn tier_rank(tier: WarningTier) -> u8 {
    match tier {
        WarningTier::Critical => 0,
        WarningTier::Actionable => 1,
        WarningTier::Noise => 2,
    }
}

fn severity_rank(severity: &WarningSeverity) -> u8 {
    match severity {
        WarningSeverity::Error => 0,
        WarningSeverity::Warning => 1,
        WarningSeverity::Info => 2,
    }
}
