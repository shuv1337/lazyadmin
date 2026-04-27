use crate::model::DOCTOR_SCHEMA_VERSION;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DoctorSeverity {
    Ok,
    Info,
    Warning,
    Degraded,
    Error,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorCheck {
    pub subsystem: String,
    pub name: String,
    pub severity: DoctorSeverity,
    pub summary: String,
    pub hint: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorReport {
    pub schema_version: String,
    pub generated_at: DateTime<Utc>,
    pub checks: Vec<DoctorCheck>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subsystems: Option<DoctorSubsystems>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DoctorSubsystems {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub adapters: Option<DoctorAdapters>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub events: Option<DoctorEvents>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DoctorAdapters {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sockets: Option<DoctorSockets>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorSockets {
    pub preferred: String,
    pub active: String,
    pub degraded: bool,
    pub parity_diff_count: u64,
    pub dual_stack_probe: DualStackProbeReport,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub struct DualStackProbeReport {
    pub supported: bool,
    pub attempted: u64,
    pub succeeded: u64,
    pub errors: u64,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorEvents {
    pub enabled: bool,
    pub per_adapter: Vec<DoctorAdapterWatch>,
    pub dropped: u64,
    #[serde(default)]
    pub drop_counter_observable: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub drop_counter_source: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DoctorAdapterWatch {
    pub adapter: String,
    pub state: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub last_event_at: Option<DateTime<Utc>>,
    pub dropped: u64,
}

impl DoctorReport {
    pub fn new(checks: Vec<DoctorCheck>) -> Self {
        Self {
            schema_version: DOCTOR_SCHEMA_VERSION.into(),
            generated_at: Utc::now(),
            checks,
            subsystems: None,
        }
    }

    pub fn with_subsystems(mut self, subsystems: DoctorSubsystems) -> Self {
        self.subsystems = Some(subsystems);
        self
    }
}
