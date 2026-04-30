use super::*;
use lazyadmin_core::model::{ActionId, EntityRef, Snapshot, Warning, WarningSeverity};

#[test]
fn empty_snapshot_projects_all_view_models() {
    let snapshot = Snapshot::empty();
    let models = RuntimeViewModels::from_snapshot(&snapshot);
    assert_eq!(models.digest.exposed.rows.len(), 0);
    assert_eq!(models.digest.conflicts.rows.len(), 0);
    assert_eq!(models.doctor_groups.groups, Vec::new());
    assert_eq!(models.header_pip.adapters.total, 0);
}

#[test]
fn warning_groups_rank_critical_before_noise() {
    let mut snapshot = Snapshot::empty();
    snapshot.warnings.push(Warning {
        severity: WarningSeverity::Warning,
        code: "possible_dual_stack".into(),
        message: "maybe dual stack".into(),
        entity: None,
        provenance: vec![],
    });
    snapshot.warnings.push(Warning {
        severity: WarningSeverity::Warning,
        code: "CONFLICT".into(),
        message: "conflict".into(),
        entity: None,
        provenance: vec![],
    });
    let groups = warning_groups(&snapshot);
    assert_eq!(groups[0].code, "CONFLICT");
    assert_eq!(groups[0].count, 1);
    assert_eq!(groups[1].code, "possible_dual_stack");
    assert!(!groups[1].expanded);
}

#[test]
fn groups_many_fd_permission_denied_into_one_noise_row() {
    let mut snapshot = Snapshot::empty();
    for _ in 0..2_183 {
        snapshot.warnings.push(Warning {
            severity: WarningSeverity::Warning,
            code: "fd_permission_denied".into(),
            message: "permission denied".into(),
            entity: None,
            provenance: vec![],
        });
    }
    let view = build_doctor_groups(&snapshot);
    assert_eq!(view.groups.len(), 1);
    assert_eq!(view.groups[0].count, 2_183);
    assert_eq!(view.noise_group_count, 1);
    assert_eq!(view.noise_total_count, 2_183);
}

#[test]
fn sample_entities_are_capped_at_five_and_stable() {
    let mut snapshot = Snapshot::empty();
    for index in 0..7 {
        snapshot.warnings.push(Warning {
            severity: WarningSeverity::Warning,
            code: "PUBLIC".into(),
            message: "public listener".into(),
            entity: Some(EntityRef::Action(ActionId::new(format!("action:{index}")))),
            provenance: vec![],
        });
    }
    let view = build_doctor_groups(&snapshot);
    assert_eq!(view.groups.len(), 1);
    assert_eq!(view.groups[0].sample_entities.len(), 5);
    assert_eq!(
        view.groups[0].sample_entities[0],
        EntityRef::Action(ActionId::new("action:0"))
    );
    assert_eq!(
        view.groups[0].sample_entities[4],
        EntityRef::Action(ActionId::new("action:4"))
    );
}
