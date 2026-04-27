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
}

impl DoctorReport {
    pub fn new(checks: Vec<DoctorCheck>) -> Self {
        Self {
            schema_version: DOCTOR_SCHEMA_VERSION.into(),
            generated_at: Utc::now(),
            checks,
        }
    }
}
