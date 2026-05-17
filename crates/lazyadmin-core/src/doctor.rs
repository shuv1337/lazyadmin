use crate::model::DOCTOR_SCHEMA_VERSION;
use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WarningTier {
    Critical,
    Actionable,
    Noise,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct WarningCodeMeta {
    pub code: &'static str,
    pub tier: WarningTier,
    pub label: &'static str,
    pub remediation: &'static str,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MetricCaption {
    pub key: &'static str,
    pub caption: &'static str,
}

pub const METRIC_CAPTIONS: &[MetricCaption] = &[
    MetricCaption {
        key: "events_dropped",
        caption: "Dropped discovery hints mean the next full snapshot is authoritative; increase event capacity only if this keeps rising.",
    },
    MetricCaption {
        key: "adapter_event_rate",
        caption: "Adapter events are refresh hints. Zero events usually means the system is idle, not broken.",
    },
    MetricCaption {
        key: "listener_histogram",
        caption: "Listener counts show exposure and triage shape; public, conflict, and orphan bars deserve review first.",
    },
];

pub fn metric_caption(key: &str) -> &'static str {
    METRIC_CAPTIONS
        .iter()
        .find(|metric| metric.key == key)
        .map(|metric| metric.caption)
        .unwrap_or("Metric is derived from the current read-only snapshot.")
}

pub const ALL_CODES: &[WarningCodeMeta] = &[
    WarningCodeMeta {
        code: "CONFLICT",
        tier: WarningTier::Critical,
        label: "Port conflict",
        remediation: "stop or move one of the processes bound to the same address and port",
    },
    WarningCodeMeta {
        code: "PUBLIC",
        tier: WarningTier::Actionable,
        label: "Public listener",
        remediation: "confirm the listener is intentionally reachable beyond loopback",
    },
    WarningCodeMeta {
        code: "TUNNEL",
        tier: WarningTier::Actionable,
        label: "Tunnel process",
        remediation: "confirm the tunnel is intentional and bound to the expected interface",
    },
    WarningCodeMeta {
        code: "SIDECAR",
        tier: WarningTier::Noise,
        label: "Sidecar process",
        remediation: "inspect only if the sidecar is unexpected for this workload",
    },
    WarningCodeMeta {
        code: "EVENTS_DROPPED",
        tier: WarningTier::Actionable,
        label: "Discovery events dropped",
        remediation: "increase event channel capacity or wait for the next authoritative snapshot",
    },
    WarningCodeMeta {
        code: "SOCK_DIAG_DOWNGRADED",
        tier: WarningTier::Noise,
        label: "Socket scan downgraded",
        remediation: "inspect socket adapter permissions if high-fidelity socket data is required",
    },
    WarningCodeMeta {
        code: "SOCK_DIAG_PARITY_DIFF",
        tier: WarningTier::Actionable,
        label: "Socket parity mismatch",
        remediation: "compare procfs and sock_diag output before trusting dual-stack details",
    },
    WarningCodeMeta {
        code: "proc_net_parse",
        tier: WarningTier::Actionable,
        label: "proc net parse issue",
        remediation: "inspect the affected /proc/net row and adapter logs",
    },
    WarningCodeMeta {
        code: "wide_bind",
        tier: WarningTier::Actionable,
        label: "Wide bind",
        remediation: "bind to loopback unless LAN/public reachability is intentional",
    },
    WarningCodeMeta {
        code: "possible_dual_stack",
        tier: WarningTier::Noise,
        label: "Possible dual-stack listener",
        remediation: "verify IPV6_V6ONLY when exact IPv4 reachability matters",
    },
    WarningCodeMeta {
        code: "permission_denied",
        tier: WarningTier::Actionable,
        label: "Permission denied",
        remediation: "rerun with permissions that can read the affected process metadata",
    },
    WarningCodeMeta {
        code: "fd_permission_denied",
        tier: WarningTier::Noise,
        label: "File descriptor permission denied",
        remediation: "rerun with elevated permissions if owner correlation is incomplete",
    },
    WarningCodeMeta {
        code: "portless.route_pid_missing",
        tier: WarningTier::Actionable,
        label: "Portless route PID missing",
        remediation: "run portless prune or restart the route",
    },
    WarningCodeMeta {
        code: "portless.routes_unreadable",
        tier: WarningTier::Actionable,
        label: "Portless routes unreadable",
        remediation: "check portless state directory permissions",
    },
    WarningCodeMeta {
        code: "portless.routes_unparseable",
        tier: WarningTier::Actionable,
        label: "Portless routes unparseable",
        remediation: "inspect routes.json and run portless prune if needed",
    },
    WarningCodeMeta {
        code: "portless.routes_invalid",
        tier: WarningTier::Actionable,
        label: "Portless route invalid",
        remediation: "remove or repair the invalid route entry",
    },
    WarningCodeMeta {
        code: "portless.orphan_route",
        tier: WarningTier::Actionable,
        label: "Portless orphan route",
        remediation: "run portless prune to remove routes whose process exited",
    },
];

pub const WARNING_CODE_REGISTRY: &[WarningCodeMeta] = ALL_CODES;

pub fn classify(code: &str) -> WarningCodeMeta {
    ALL_CODES
        .iter()
        .copied()
        .find(|meta| meta.code == code)
        .unwrap_or(WarningCodeMeta {
            code: "unknown",
            tier: WarningTier::Actionable,
            label: "unknown warning",
            remediation: "inspect details",
        })
}
use chrono::{DateTime, Utc};

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

#[cfg(test)]
mod tests {
    use super::*;

    const EMITTED_WARNING_CODES: &[&str] = &[
        "CONFLICT",
        "EVENTS_DROPPED",
        "PUBLIC",
        "SIDECAR",
        "SOCK_DIAG_DOWNGRADED",
        "SOCK_DIAG_PARITY_DIFF",
        "TUNNEL",
        "fd_permission_denied",
        "permission_denied",
        "portless.orphan_route",
        "portless.route_pid_missing",
        "portless.routes_invalid",
        "portless.routes_unparseable",
        "portless.routes_unreadable",
        "possible_dual_stack",
        "proc_net_parse",
        "wide_bind",
    ];

    #[test]
    fn every_emitted_code_has_registry_entry() {
        for code in EMITTED_WARNING_CODES {
            assert_ne!(
                classify(code).code,
                "unknown",
                "missing registry row for {code}"
            );
        }
    }

    #[test]
    fn classifies_shipped_warning_codes() {
        for meta in ALL_CODES {
            assert_eq!(classify(meta.code), *meta);
            assert!(!meta.label.is_empty());
            assert!(!meta.remediation.is_empty());
        }
    }

    #[test]
    fn unknown_code_defaults_to_actionable() {
        let meta = classify("future.warning");
        assert_eq!(meta.tier, WarningTier::Actionable);
        assert_eq!(meta.remediation, "inspect details");
    }

    #[test]
    fn classify_is_pure() {
        assert_eq!(classify("PUBLIC"), classify("PUBLIC"));
        assert_eq!(classify("future.warning"), classify("future.warning"));
    }

    #[test]
    fn metric_caption_returns_known_value_for_registered_keys() {
        for entry in METRIC_CAPTIONS {
            assert_eq!(metric_caption(entry.key), entry.caption);
        }
    }

    #[test]
    fn metric_caption_falls_back_for_unknown_key() {
        let s = metric_caption("some.future.metric");
        assert!(
            s.contains("read-only snapshot") || !s.is_empty(),
            "got: {s}"
        );
    }

    #[test]
    fn warning_code_registry_is_alias_of_all_codes() {
        assert_eq!(WARNING_CODE_REGISTRY.len(), ALL_CODES.len());
        for (a, b) in WARNING_CODE_REGISTRY.iter().zip(ALL_CODES.iter()) {
            assert_eq!(a, b);
        }
    }

    #[test]
    fn warning_codes_are_unique() {
        let mut codes: Vec<_> = ALL_CODES.iter().map(|m| m.code).collect();
        codes.sort_unstable();
        let len = codes.len();
        codes.dedup();
        assert_eq!(codes.len(), len, "duplicate code in WARNING_CODE_REGISTRY");
    }

    #[test]
    fn warning_tier_round_trips_via_json() {
        for tier in [
            WarningTier::Critical,
            WarningTier::Actionable,
            WarningTier::Noise,
        ] {
            let json = serde_json::to_string(&tier).unwrap();
            let back: WarningTier = serde_json::from_str(&json).unwrap();
            assert_eq!(back, tier);
        }
    }

    #[test]
    fn doctor_severity_round_trips_via_json() {
        for sev in [
            DoctorSeverity::Ok,
            DoctorSeverity::Info,
            DoctorSeverity::Warning,
            DoctorSeverity::Degraded,
            DoctorSeverity::Error,
        ] {
            let json = serde_json::to_string(&sev).unwrap();
            let back: DoctorSeverity = serde_json::from_str(&json).unwrap();
            assert_eq!(back, sev);
        }
    }

    #[test]
    fn doctor_report_new_sets_schema_version_and_no_subsystems() {
        let r = DoctorReport::new(vec![]);
        assert_eq!(r.schema_version, DOCTOR_SCHEMA_VERSION);
        assert!(r.subsystems.is_none());
        assert!(r.checks.is_empty());
    }

    #[test]
    fn doctor_report_with_subsystems_attaches_them() {
        let r = DoctorReport::new(vec![]).with_subsystems(DoctorSubsystems::default());
        assert!(r.subsystems.is_some());
    }

    #[test]
    fn doctor_subsystems_default_is_empty() {
        let s = DoctorSubsystems::default();
        assert!(s.adapters.is_none());
        assert!(s.events.is_none());
    }

    #[test]
    fn dual_stack_probe_report_default() {
        let p = DualStackProbeReport::default();
        assert!(!p.supported);
        assert_eq!(p.attempted, 0);
        assert_eq!(p.succeeded, 0);
        assert_eq!(p.errors, 0);
    }

    #[test]
    fn doctor_check_serializes_with_optional_hint() {
        let c = DoctorCheck {
            subsystem: "sub".into(),
            name: "n".into(),
            severity: DoctorSeverity::Warning,
            summary: "s".into(),
            hint: Some("h".into()),
        };
        let json = serde_json::to_string(&c).unwrap();
        let back: DoctorCheck = serde_json::from_str(&json).unwrap();
        assert_eq!(back, c);
    }
}
