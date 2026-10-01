use std::time::Duration;

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use super::SystemState;

/// Structured health finding. Not a free-form string.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum HealthReason {
    MarketDataStale {
        token_id: String,
        age_ms: u64,
        max_age_ms: u64,
    },
    MarketDataUnknown,
    ConnectionMissing,
    ConnectionDisconnected,
    ConnectionReconnecting,
    ConnectionStale,
    ConnectionFailed,
    ConnectedWithoutEvents,
    ReconnectLoop {
        count: u32,
    },
    PositionMismatch {
        token_id: String,
    },
    PositionUnknown,
    ExecutionUnknown,
    ExecutionInconsistent,
    ExecutionNotChecked,
    ReconciliationIncomplete,
    ReconciliationFailed,
    ReconciliationMismatch,
    RiskWarning,
    RiskBreached {
        rule: String,
    },
    RiskUnknown,
    PersistenceUnhealthy,
    RecoveryInProgress,
    RecoveryRequired,
    KillSwitchActive,
    OperatorPaused,
    CriticalAlert,
}

impl HealthReason {
    pub fn code(&self) -> &'static str {
        match self {
            Self::MarketDataStale { .. } => "MARKET_DATA_STALE",
            Self::MarketDataUnknown => "MARKET_DATA_UNKNOWN",
            Self::ConnectionMissing => "CONNECTION_MISSING",
            Self::ConnectionDisconnected => "WEBSOCKET_DISCONNECTED",
            Self::ConnectionReconnecting => "WEBSOCKET_RECONNECTING",
            Self::ConnectionStale => "WEBSOCKET_STALE",
            Self::ConnectionFailed => "WEBSOCKET_FAILED",
            Self::ConnectedWithoutEvents => "CONNECTED_WITHOUT_EVENTS",
            Self::ReconnectLoop { .. } => "RECONNECT_LOOP",
            Self::PositionMismatch { .. } => "POSITION_MISMATCH",
            Self::PositionUnknown => "POSITION_UNKNOWN",
            Self::ExecutionUnknown => "EXECUTION_UNKNOWN",
            Self::ExecutionInconsistent => "EXECUTION_INCONSISTENT",
            Self::ExecutionNotChecked => "EXECUTION_NOT_CHECKED",
            Self::ReconciliationIncomplete => "RECONCILIATION_INCOMPLETE",
            Self::ReconciliationFailed => "RECONCILIATION_FAILED",
            Self::ReconciliationMismatch => "STATE_MISMATCH",
            Self::RiskWarning => "RISK_WARNING",
            Self::RiskBreached { .. } => "RISK_LIMIT_BREACHED",
            Self::RiskUnknown => "RISK_UNKNOWN",
            Self::PersistenceUnhealthy => "PERSISTENCE_UNHEALTHY",
            Self::RecoveryInProgress => "RECOVERY_IN_PROGRESS",
            Self::RecoveryRequired => "RECOVERY_REQUIRED",
            Self::KillSwitchActive => "KILL_SWITCH_TRIGGERED",
            Self::OperatorPaused => "TRADING_PAUSED",
            Self::CriticalAlert => "CRITICAL_ALERT",
        }
    }
}

impl std::fmt::Display for HealthReason {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::MarketDataStale {
                token_id,
                age_ms,
                max_age_ms,
            } => write!(
                f,
                "MARKET_DATA_STALE token={token_id} age_ms={age_ms} max_age_ms={max_age_ms}"
            ),
            Self::PositionMismatch { token_id } => {
                write!(f, "POSITION_MISMATCH token={token_id}")
            }
            Self::RiskBreached { rule } => write!(f, "RISK_LIMIT_BREACHED rule={rule}"),
            Self::ReconnectLoop { count } => write!(f, "RECONNECT_LOOP count={count}"),
            other => f.write_str(other.code()),
        }
    }
}

/// Result of a health evaluation.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SystemHealth {
    pub state: SystemState,
    pub reasons: Vec<HealthReason>,
    pub evaluated_at: DateTime<Utc>,
}

/// Transport health. `Connected` is not the same as system `Healthy`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConnectionHealth {
    Connected,
    Disconnected,
    Reconnecting,
    Stale,
    Failed,
}

impl std::fmt::Display for ConnectionHealth {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Connected => "CONNECTED",
            Self::Disconnected => "DISCONNECTED",
            Self::Reconnecting => "RECONNECTING",
            Self::Stale => "STALE",
            Self::Failed => "FAILED",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Freshness {
    Fresh,
    Stale,
    Unknown,
}

impl std::fmt::Display for Freshness {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Fresh => "FRESH",
            Self::Stale => "STALE",
            Self::Unknown => "UNKNOWN",
        })
    }
}

pub fn age_ms(now: DateTime<Utc>, then: DateTime<Utc>) -> u64 {
    now.signed_duration_since(then).num_milliseconds().max(0) as u64
}

pub fn exceeds(age: Duration, max_age: Duration) -> bool {
    age > max_age
}
