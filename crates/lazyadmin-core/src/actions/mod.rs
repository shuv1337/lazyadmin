#![allow(clippy::module_name_repetitions)]

use crate::model::{ActionId, DangerLevel, EntityRef, ProcessKey, RuntimeKind, Snapshot};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::time::Duration;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    Stop,
    Restart,
    Kill,
    FreePort,
    PortlessStop,
    PauseRestart,
    ResumeRestart,
    Logs,
    Forget,
    SignalProcessGroup,
    SignalPid,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Requirement {
    RuntimeAvailable { runtime: RuntimeKind },
    Permission { detail: String },
    PolkitOrSudo { detail: String },
    TypedPhrase { phrase: String },
    SelectorDisambiguation,
    RestartPolicyPauseRecommended { policy: String },
    ProcessKeyMatch { key: ProcessKey },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", tag = "kind")]
pub enum ConfirmationPolicy {
    None,
    YesNo,
    TypedPhrase { phrase: String },
    TestOnlyBypass,
}

impl ConfirmationPolicy {
    #[must_use]
    pub fn render_prompt(&self) -> String {
        match self {
            Self::None => "no confirmation required".into(),
            Self::YesNo => "Continue? [y/N]".into(),
            Self::TypedPhrase { phrase } => format!("Type \"{phrase}\" to continue:"),
            Self::TestOnlyBypass => "TEST-ONLY confirmation bypass in effect".into(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DryRunLine {
    pub summary: String,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Action {
    pub id: ActionId,
    pub label: String,
    pub kind: ActionKind,
    pub danger: DangerLevel,
    pub requirements: Vec<Requirement>,
    pub dry_run: Vec<DryRunLine>,
    pub target: EntityRef,
    pub runtime: RuntimeKind,
    pub confirmation: ConfirmationPolicy,
    pub timeout_ms: u64,
    pub provenance: Vec<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionPlan {
    pub id: String,
    pub created_at: DateTime<Utc>,
    pub target: String,
    pub actions: Vec<Action>,
    pub dry_run: Vec<DryRunLine>,
    pub confirmation: ConfirmationPolicy,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionStatus {
    Success,
    Failed,
    TimedOut,
    Skipped,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionResult {
    pub action_id: ActionId,
    pub status: ActionStatus,
    pub message: String,
    pub duration_ms: u128,
    pub error_class: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionExecutionReport {
    pub schema_version: String,
    pub plan: ActionPlan,
    pub results: Vec<ActionResult>,
    pub before_summary: String,
    pub after_summary: String,
    pub diff_summaries: Vec<String>,
}

pub trait ActionPlanner {
    fn plan(&self, target: &EntityRef, graph: &Snapshot) -> Vec<Action>;
}

pub trait ActionExecutor {
    fn execute(&self, action: &Action, timeout: Duration) -> ActionResult;
}

#[derive(Clone, Debug, Default)]
pub struct ActionPlanTelemetry {
    pub action_count: usize,
}

pub fn render_dry_run(lines: &[DryRunLine]) -> String {
    lines
        .iter()
        .map(|l| match &l.detail {
            Some(d) => format!("- {} ({d})", l.summary),
            None => format!("- {}", l.summary),
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn confirmation_policy_rendering() {
        assert!(
            ConfirmationPolicy::TypedPhrase {
                phrase: "free".into()
            }
            .render_prompt()
            .contains("free")
        );
    }
    #[test]
    fn dry_run_stable_output() {
        assert_eq!(
            render_dry_run(&[DryRunLine {
                summary: "stop pid".into(),
                detail: Some("SIGTERM".into())
            }]),
            "- stop pid (SIGTERM)"
        );
    }
    #[test]
    fn action_serialization() {
        let p = ActionPlan {
            id: "p".into(),
            created_at: Utc::now(),
            target: ":1".into(),
            actions: vec![],
            dry_run: vec![],
            confirmation: ConfirmationPolicy::YesNo,
        };
        serde_json::to_string(&p).unwrap();
    }

    #[test]
    fn confirmation_none_yes_no_and_bypass_render_distinct_prompts() {
        let none = ConfirmationPolicy::None.render_prompt();
        let yn = ConfirmationPolicy::YesNo.render_prompt();
        let bypass = ConfirmationPolicy::TestOnlyBypass.render_prompt();
        assert_ne!(none, yn);
        assert_ne!(yn, bypass);
        assert!(none.contains("no confirmation"));
        assert!(yn.contains("[y/N]"));
        assert!(bypass.contains("TEST-ONLY"));
    }

    #[test]
    fn typed_phrase_prompt_quotes_the_phrase() {
        let p = ConfirmationPolicy::TypedPhrase {
            phrase: "DELETE".into(),
        };
        let s = p.render_prompt();
        assert!(s.contains("\"DELETE\""));
    }

    #[test]
    fn render_dry_run_handles_lines_without_detail() {
        let out = render_dry_run(&[
            DryRunLine {
                summary: "plan a".into(),
                detail: None,
            },
            DryRunLine {
                summary: "plan b".into(),
                detail: Some("SIGKILL".into()),
            },
        ]);
        assert_eq!(out, "- plan a\n- plan b (SIGKILL)");
    }

    #[test]
    fn render_dry_run_empty_returns_empty_string() {
        assert_eq!(render_dry_run(&[]), "");
    }

    #[test]
    fn action_kind_round_trips_via_json_snake_case() {
        let cases = [
            (ActionKind::Stop, "stop"),
            (ActionKind::Restart, "restart"),
            (ActionKind::Kill, "kill"),
            (ActionKind::FreePort, "free_port"),
            (ActionKind::PortlessStop, "portless_stop"),
            (ActionKind::PauseRestart, "pause_restart"),
            (ActionKind::ResumeRestart, "resume_restart"),
            (ActionKind::Logs, "logs"),
            (ActionKind::Forget, "forget"),
            (ActionKind::SignalProcessGroup, "signal_process_group"),
            (ActionKind::SignalPid, "signal_pid"),
            (ActionKind::Unsupported, "unsupported"),
        ];
        for (kind, expected) in cases {
            let json = serde_json::to_string(&kind).unwrap();
            assert_eq!(json, format!("\"{expected}\""));
            let back: ActionKind = serde_json::from_str(&json).unwrap();
            assert_eq!(back, kind);
        }
    }

    #[test]
    fn action_status_round_trips_via_json_snake_case() {
        for s in [
            ActionStatus::Success,
            ActionStatus::Failed,
            ActionStatus::TimedOut,
            ActionStatus::Skipped,
            ActionStatus::Unsupported,
        ] {
            let json = serde_json::to_string(&s).unwrap();
            let back: ActionStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(back, s);
        }
    }

    #[test]
    fn requirement_serializes_with_kind_and_detail() {
        let r = Requirement::Permission {
            detail: "need CAP_SYS_PTRACE".into(),
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: Requirement = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }

    #[test]
    fn action_plan_telemetry_default_is_zero() {
        let t = ActionPlanTelemetry::default();
        assert_eq!(t.action_count, 0);
    }

    #[test]
    fn action_result_round_trips_with_error_class() {
        let r = ActionResult {
            action_id: ActionId::new("a-1"),
            status: ActionStatus::Failed,
            message: "timed out".into(),
            duration_ms: 1500,
            error_class: Some("io::ErrorKind::TimedOut".into()),
        };
        let json = serde_json::to_string(&r).unwrap();
        let back: ActionResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back, r);
    }
}
