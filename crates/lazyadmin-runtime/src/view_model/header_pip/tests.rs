use super::*;
use chrono::{Duration, TimeZone, Utc};
use lazyadmin_core::model::{
    Manager, ManagerId, ManagerScope, PermissionState, RuntimeKind, Snapshot, SnapshotMetadata,
};

fn mgr(id: &str, available: bool, permission: PermissionState) -> Manager {
    Manager {
        id: ManagerId::new(id),
        kind: RuntimeKind::Docker,
        name: id.into(),
        scope: ManagerScope::System,
        socket: None,
        available,
        permission,
        version: None,
        provenance: vec![],
    }
}

#[test]
fn empty_snapshot_yields_zero_counts() {
    let snapshot = Snapshot::empty();
    let pip = HeaderPip::from_snapshot(&snapshot);
    assert_eq!(pip.adapters.total, 0);
    assert_eq!(pip.adapters.active, 0);
    assert_eq!(pip.adapters.degraded, 0);
    assert!(pip.drops.is_none());
    // Generated_at is "now"-ish; age must be non-negative.
    assert!(pip.freshness.age_seconds >= 0);
}

#[test]
fn unavailable_manager_counts_as_degraded_not_active() {
    let mut snapshot = Snapshot::empty();
    snapshot.managers.push(mgr("a", false, PermissionState::Ok));
    let pip = HeaderPip::from_snapshot(&snapshot);
    assert_eq!(pip.adapters.total, 1);
    assert_eq!(pip.adapters.active, 0);
    assert_eq!(pip.adapters.degraded, 1);
}

#[test]
fn available_but_partial_permission_is_degraded() {
    let mut snapshot = Snapshot::empty();
    snapshot
        .managers
        .push(mgr("a", true, PermissionState::Partial));
    let pip = HeaderPip::from_snapshot(&snapshot);
    assert_eq!(pip.adapters.active, 1);
    assert_eq!(pip.adapters.degraded, 1, "partial permission is degraded");
}

#[test]
fn available_and_ok_is_active_and_not_degraded() {
    let mut snapshot = Snapshot::empty();
    snapshot.managers.push(mgr("a", true, PermissionState::Ok));
    let pip = HeaderPip::from_snapshot(&snapshot);
    assert_eq!(pip.adapters.active, 1);
    assert_eq!(pip.adapters.degraded, 0);
}

#[test]
fn mixed_managers_count_correctly() {
    let mut snapshot = Snapshot::empty();
    snapshot.managers.push(mgr("ok", true, PermissionState::Ok));
    snapshot
        .managers
        .push(mgr("partial", true, PermissionState::Partial));
    snapshot
        .managers
        .push(mgr("denied", true, PermissionState::Denied));
    snapshot
        .managers
        .push(mgr("offline", false, PermissionState::Ok));
    snapshot
        .managers
        .push(mgr("unknown", true, PermissionState::Unknown));
    let pip = HeaderPip::from_snapshot(&snapshot);
    assert_eq!(pip.adapters.total, 5);
    assert_eq!(pip.adapters.active, 4, "all available are active");
    // Degraded = unavailable OR permission != Ok. Of 5: partial, denied, offline, unknown = 4.
    assert_eq!(pip.adapters.degraded, 4);
}

#[test]
fn freshness_uses_snapshot_generated_at_and_clamps_negative_age() {
    let mut snapshot = Snapshot::empty();
    // Future timestamp -> would yield negative age; HeaderPip clamps to 0.
    snapshot.generated_at = Utc::now() + Duration::seconds(60);
    let pip = HeaderPip::from_snapshot(&snapshot);
    assert_eq!(pip.freshness.generated_at, snapshot.generated_at);
    assert_eq!(pip.freshness.age_seconds, 0);
}

#[test]
fn freshness_age_increases_for_past_timestamps() {
    let mut snapshot = Snapshot::empty();
    snapshot.generated_at = Utc.with_ymd_and_hms(2020, 1, 1, 0, 0, 0).unwrap();
    let pip = HeaderPip::from_snapshot(&snapshot);
    assert!(
        pip.freshness.age_seconds > 60 * 60 * 24 * 365,
        "expected old snapshot to be more than a year stale, got {}",
        pip.freshness.age_seconds
    );
}

#[test]
fn drops_none_when_events_dropped_is_zero_or_missing() {
    let mut snapshot = Snapshot::empty();
    snapshot.metadata = Some(SnapshotMetadata {
        events_dropped: Some(0),
    });
    assert!(HeaderPip::from_snapshot(&snapshot).drops.is_none());

    snapshot.metadata = Some(SnapshotMetadata {
        events_dropped: None,
    });
    assert!(HeaderPip::from_snapshot(&snapshot).drops.is_none());

    snapshot.metadata = None;
    assert!(HeaderPip::from_snapshot(&snapshot).drops.is_none());
}

#[test]
fn drops_some_when_events_dropped_positive() {
    let mut snapshot = Snapshot::empty();
    snapshot.metadata = Some(SnapshotMetadata {
        events_dropped: Some(7),
    });
    let pip = HeaderPip::from_snapshot(&snapshot);
    assert_eq!(pip.drops, Some(DropRate { dropped: 7 }));
}

#[test]
fn header_pip_round_trips_through_json() {
    let mut snapshot = Snapshot::empty();
    snapshot.managers.push(mgr("a", true, PermissionState::Ok));
    snapshot.metadata = Some(SnapshotMetadata {
        events_dropped: Some(3),
    });
    let pip = HeaderPip::from_snapshot(&snapshot);
    let json = serde_json::to_string(&pip).unwrap();
    let back: HeaderPip = serde_json::from_str(&json).unwrap();
    assert_eq!(back, pip);
}

#[test]
fn adapter_health_default_is_zeros() {
    let h = AdapterHealth::default();
    assert_eq!(h.active, 0);
    assert_eq!(h.total, 0);
    assert_eq!(h.degraded, 0);
}
