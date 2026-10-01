use serde::{Deserialize, Serialize};

/// Explicit trading permission. Connectivity alone never implies this value.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TradingPermission {
    TradingAllowed,
    TradingBlocked,
    TradingRequiresVerification,
}

impl TradingPermission {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::TradingAllowed => "TRADING_ALLOWED",
            Self::TradingBlocked => "TRADING_BLOCKED",
            Self::TradingRequiresVerification => "TRADING_REQUIRES_VERIFICATION",
        }
    }

    pub const fn allows_new_trading(self) -> bool {
        matches!(self, Self::TradingAllowed)
    }
}

impl std::fmt::Display for TradingPermission {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Answer returned to a trading bot that asks whether it may trade.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum TradeDecision {
    Allow,
    Block,
    RequireReconciliation,
    RequireRecovery,
}

impl TradeDecision {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allow => "ALLOW",
            Self::Block => "BLOCK",
            Self::RequireReconciliation => "REQUIRE_RECONCILIATION",
            Self::RequireRecovery => "REQUIRE_RECOVERY",
        }
    }
}

impl std::fmt::Display for TradeDecision {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

/// Structured decision for the trading-bot integration boundary.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TradeGate {
    pub decision: TradeDecision,
    pub permission: TradingPermission,
    pub system_state: super::SystemState,
    pub reasons: Vec<super::HealthReason>,
}
