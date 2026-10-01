use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Top-level operational state of the trading system.
///
/// `Killed` is the kill-switch latch. It is stronger than [`SystemState::Paused`]
/// and is not cleared by resume, reconnect, or recovery.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SystemState {
    Healthy,
    Degraded,
    Paused,
    Recovering,
    Failed,
    Killed,
}

impl SystemState {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Healthy => "HEALTHY",
            Self::Degraded => "DEGRADED",
            Self::Paused => "PAUSED",
            Self::Recovering => "RECOVERING",
            Self::Failed => "FAILED",
            Self::Killed => "KILLED",
        }
    }
}

impl std::fmt::Display for SystemState {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Who requested a state transition.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Actor {
    System,
    RiskEngine,
    HealthEngine,
    Reconciler,
    Operator,
    RecoveryEngine,
}

impl std::fmt::Display for Actor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::System => "SYSTEM",
            Self::RiskEngine => "RISK_ENGINE",
            Self::HealthEngine => "HEALTH_ENGINE",
            Self::Reconciler => "RECONCILER",
            Self::Operator => "OPERATOR",
            Self::RecoveryEngine => "RECOVERY_ENGINE",
        })
    }
}

/// Why a transition was attempted. Authoritative reason codes, not free-form text.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TransitionReason {
    StartupUnverified,
    ChecksPassed,
    MarketDataStale,
    MarketDataUnknown,
    PositionMismatch,
    PositionUnknown,
    ExecutionUnknown,
    ExecutionInconsistent,
    RiskLimitBreached,
    RiskUnknown,
    DailyLossKill,
    WebsocketDisconnected,
    WebsocketReconnected,
    ConnectionUnhealthy,
    ReconnectLoop,
    RecoveryStarted,
    RecoveryCompleted,
    RecoveryFailed,
    OperatorPause,
    OperatorResume,
    OperatorKill,
    ReconciliationRequired,
    PersistenceFailure,
    EventGap,
    CriticalAlert,
    DegradedWarning,
}

impl std::fmt::Display for TransitionReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        let name = format!("{self:?}");
        let mut out = String::new();
        for (i, ch) in name.chars().enumerate() {
            if ch.is_uppercase() && i > 0 {
                out.push('_');
            }
            out.push(ch.to_ascii_uppercase());
        }
        f.write_str(&out)
    }
}

/// How a transition was requested. The engine uses this together with policy gates.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TransitionKind {
    Automatic,
    Pause,
    Resume,
    Kill,
    RecoveryStart,
    RecoveryComplete,
}

/// Conditions the transition function consults. Enum changes alone are not enough.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct TransitionContext {
    pub gates_open: bool,
    pub kill_latched: bool,
    pub operator_paused: bool,
    pub recovery_required: bool,
    pub recovery_active: bool,
}

/// An observed, persisted state change.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StateTransition {
    pub previous: SystemState,
    pub next: SystemState,
    pub reason: TransitionReason,
    pub kind: TransitionKind,
    pub actor: Actor,
    pub at: DateTime<Utc>,
    pub detail: String,
}

/// Operator and engine commands.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Command {
    Pause {
        reason: String,
    },
    Resume,
    Kill {
        reason: String,
    },
    Reconcile,
    Recover {
        trigger: crate::domain::RecoveryTrigger,
    },
}
