use chrono::{DateTime, Utc};
use lazyadmin_core::model::{PermissionState, Snapshot};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterHealth {
    pub active: usize,
    pub total: usize,
    pub degraded: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotFreshness {
    pub generated_at: DateTime<Utc>,
    pub age_seconds: i64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DropRate {
    pub dropped: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HeaderPip {
    pub adapters: AdapterHealth,
    pub freshness: SnapshotFreshness,
    pub drops: Option<DropRate>,
}

impl HeaderPip {
    pub fn from_snapshot(snapshot: &Snapshot) -> Self {
        let total = snapshot.managers.len();
        let active = snapshot
            .managers
            .iter()
            .filter(|manager| manager.available)
            .count();
        let degraded = snapshot
            .managers
            .iter()
            .filter(|manager| !manager.available || manager.permission != PermissionState::Ok)
            .count();
        let now = Utc::now();
        Self {
            adapters: AdapterHealth {
                active,
                total,
                degraded,
            },
            freshness: SnapshotFreshness {
                generated_at: snapshot.generated_at,
                age_seconds: now
                    .signed_duration_since(snapshot.generated_at)
                    .num_seconds()
                    .max(0),
            },
            drops: snapshot
                .metadata
                .as_ref()
                .and_then(|metadata| metadata.events_dropped)
                .filter(|dropped| *dropped > 0)
                .map(|dropped| DropRate { dropped }),
        }
    }
}
