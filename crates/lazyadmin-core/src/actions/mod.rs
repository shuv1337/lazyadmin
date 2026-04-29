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
}
