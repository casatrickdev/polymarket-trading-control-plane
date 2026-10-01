use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use super::Severity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AlertType {
    StateMismatch,
    OrderFailure,
    PartialFill,
    MarketDataStale,
    WebsocketDisconnected,
    ReconciliationStarted,
    ReconciliationCompleted,
    RiskLimitBreached,
    TradingPaused,
    KillSwitchTriggered,
}

impl AlertType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::StateMismatch => "STATE_MISMATCH",
            Self::OrderFailure => "ORDER_FAILURE",
            Self::PartialFill => "PARTIAL_FILL",
            Self::MarketDataStale => "MARKET_DATA_STALE",
            Self::WebsocketDisconnected => "WEBSOCKET_DISCONNECTED",
            Self::ReconciliationStarted => "RECONCILIATION_STARTED",
            Self::ReconciliationCompleted => "RECONCILIATION_COMPLETED",
            Self::RiskLimitBreached => "RISK_LIMIT_BREACHED",
            Self::TradingPaused => "TRADING_PAUSED",
            Self::KillSwitchTriggered => "KILL_SWITCH_TRIGGERED",
        }
    }

    /// Persistent conditions are deduplicated while unresolved.
    /// Point-in-time events are recorded once per occurrence.
    pub const fn is_condition(self) -> bool {
        matches!(
            self,
            Self::StateMismatch
                | Self::MarketDataStale
                | Self::WebsocketDisconnected
                | Self::RiskLimitBreached
                | Self::TradingPaused
                | Self::KillSwitchTriggered
        )
    }
}

impl std::fmt::Display for AlertType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Alert {
    pub id: String,
    pub event_type: AlertType,
    pub severity: Severity,
    pub at: DateTime<Utc>,
    pub scope: String,
    pub entity_id: Option<String>,
    pub message: String,
    pub metadata: Value,
    pub resolved: bool,
    pub resolved_at: Option<DateTime<Utc>>,
    pub dedup_key: String,
}

impl Alert {
    pub fn condition_key(event_type: AlertType, scope: &str, entity_id: Option<&str>) -> String {
        format!(
            "{}:{}:{}",
            event_type.as_str(),
            scope,
            entity_id.unwrap_or("-")
        )
    }

    pub fn is_critical_unresolved(&self) -> bool {
        !self.resolved && self.severity == Severity::Critical
    }
}
