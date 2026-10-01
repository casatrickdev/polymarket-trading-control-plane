use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RecoveryTrigger {
    WebsocketReconnect,
    ApplicationRestart,
    SuspectedEventGap,
    UnexpectedApiResponse,
    StateMismatch,
    StaleState,
    ExecutionUncertainty,
    Operator,
}

impl std::fmt::Display for RecoveryTrigger {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::WebsocketReconnect => "WEBSOCKET_RECONNECT",
            Self::ApplicationRestart => "APPLICATION_RESTART",
            Self::SuspectedEventGap => "SUSPECTED_EVENT_GAP",
            Self::UnexpectedApiResponse => "UNEXPECTED_API_RESPONSE",
            Self::StateMismatch => "STATE_MISMATCH",
            Self::StaleState => "STALE_STATE",
            Self::ExecutionUncertainty => "EXECUTION_UNCERTAINTY",
            Self::Operator => "OPERATOR",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RecoveryStageName {
    RecoveryStarted,
    FetchAuthoritativeState,
    ReconcileOrders,
    ReconcileFills,
    ReconcilePositions,
    RecalculateExposure,
    RecheckRisk,
    RecoveryCompleted,
}

impl std::fmt::Display for RecoveryStageName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::RecoveryStarted => "RECOVERY_STARTED",
            Self::FetchAuthoritativeState => "FETCH_AUTHORITATIVE_STATE",
            Self::ReconcileOrders => "RECONCILE_ORDERS",
            Self::ReconcileFills => "RECONCILE_FILLS",
            Self::ReconcilePositions => "RECONCILE_POSITIONS",
            Self::RecalculateExposure => "RECALCULATE_EXPOSURE",
            Self::RecheckRisk => "RECHECK_RISK",
            Self::RecoveryCompleted => "RECOVERY_COMPLETED",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StageOutcome {
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoveryStageRecord {
    pub stage: RecoveryStageName,
    pub outcome: StageOutcome,
    pub at: DateTime<Utc>,
    pub detail: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RecoveryResult {
    Completed,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecoverySession {
    pub id: Uuid,
    pub started_at: DateTime<Utc>,
    pub ended_at: Option<DateTime<Utc>>,
    pub trigger: RecoveryTrigger,
    pub stages: Vec<RecoveryStageRecord>,
    pub failures: Vec<String>,
    pub result: Option<RecoveryResult>,
}

impl RecoverySession {
    pub fn start(trigger: RecoveryTrigger, now: DateTime<Utc>) -> Self {
        Self {
            id: Uuid::new_v4(),
            started_at: now,
            ended_at: None,
            trigger,
            stages: vec![RecoveryStageRecord {
                stage: RecoveryStageName::RecoveryStarted,
                outcome: StageOutcome::Succeeded,
                at: now,
                detail: trigger.to_string(),
            }],
            failures: Vec::new(),
            result: None,
        }
    }

    pub fn in_progress(&self) -> bool {
        self.result.is_none()
    }

    pub fn push(
        &mut self,
        stage: RecoveryStageName,
        outcome: StageOutcome,
        at: DateTime<Utc>,
        detail: impl Into<String>,
    ) {
        let detail = detail.into();
        if outcome == StageOutcome::Failed {
            self.failures.push(format!("{stage}: {detail}"));
        }
        self.stages.push(RecoveryStageRecord {
            stage,
            outcome,
            at,
            detail,
        });
    }

    pub fn finish(&mut self, result: RecoveryResult, at: DateTime<Utc>) {
        self.result = Some(result);
        self.ended_at = Some(at);
        let outcome = match result {
            RecoveryResult::Completed => StageOutcome::Succeeded,
            RecoveryResult::Failed => StageOutcome::Failed,
        };
        self.stages.push(RecoveryStageRecord {
            stage: RecoveryStageName::RecoveryCompleted,
            outcome,
            at,
            detail: match result {
                RecoveryResult::Completed => "recovery completed".to_string(),
                RecoveryResult::Failed => "recovery failed; trading remains paused".to_string(),
            },
        });
    }
}
