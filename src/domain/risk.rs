use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

/// Configurable risk limits. Values are never hardcoded in evaluation logic.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskConfig {
    pub max_position: Decimal,
    pub max_market_exposure: Decimal,
    pub max_total_exposure: Decimal,
    pub max_daily_loss: Decimal,
    pub max_execution_failures: u32,
    pub max_market_data_age_ms: u64,
    pub max_connection_event_age_ms: u64,
    pub reconnect_loop_threshold: u32,
    /// Fraction of a limit that raises `Warning` before `Breached`.
    pub warning_ratio: Decimal,
    pub daily_loss_action: BreachAction,
    pub default_breach_action: BreachAction,
}

impl Default for RiskConfig {
    fn default() -> Self {
        Self {
            max_position: Decimal::from(500),
            max_market_exposure: Decimal::from(1000),
            max_total_exposure: Decimal::from(5000),
            max_daily_loss: Decimal::from(250),
            max_execution_failures: 5,
            max_market_data_age_ms: 5_000,
            max_connection_event_age_ms: 5_000,
            reconnect_loop_threshold: 5,
            warning_ratio: Decimal::new(8, 1),
            daily_loss_action: BreachAction::Kill,
            default_breach_action: BreachAction::Pause,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BreachAction {
    Pause,
    Kill,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RiskStatus {
    Healthy,
    Warning,
    Breached,
    Unknown,
}

impl std::fmt::Display for RiskStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Healthy => "HEALTHY",
            Self::Warning => "WARNING",
            Self::Breached => "BREACHED",
            Self::Unknown => "UNKNOWN",
        })
    }
}

/// Rules do not all apply to every scope. The scope is part of the finding.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RiskRule {
    MaxPosition,
    MaxMarketExposure,
    MaxTotalExposure,
    MaxDailyLoss,
    MaxExecutionFailures,
    MaxMarketDataAge,
}

impl RiskRule {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::MaxPosition => "MAX_POSITION",
            Self::MaxMarketExposure => "MAX_MARKET_EXPOSURE",
            Self::MaxTotalExposure => "MAX_TOTAL_EXPOSURE",
            Self::MaxDailyLoss => "MAX_DAILY_LOSS",
            Self::MaxExecutionFailures => "MAX_EXECUTION_FAILURES",
            Self::MaxMarketDataAge => "MAX_MARKET_DATA_AGE",
        }
    }

    pub const fn action(self, config: &RiskConfig) -> BreachAction {
        match self {
            Self::MaxDailyLoss => config.daily_loss_action,
            _ => config.default_breach_action,
        }
    }
}

impl std::fmt::Display for RiskRule {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum RiskScope {
    Market { market_id: String },
    Token { token_id: String },
    Portfolio,
    System,
}

impl std::fmt::Display for RiskScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Market { market_id } => write!(f, "MARKET:{market_id}"),
            Self::Token { token_id } => write!(f, "TOKEN:{token_id}"),
            Self::Portfolio => f.write_str("PORTFOLIO"),
            Self::System => f.write_str("SYSTEM"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

impl std::fmt::Display for Severity {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Info => "INFO",
            Self::Warning => "WARNING",
            Self::Critical => "CRITICAL",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskFinding {
    pub rule: RiskRule,
    pub status: RiskStatus,
    pub scope: RiskScope,
    pub observed: Decimal,
    pub threshold: Decimal,
    pub at: DateTime<Utc>,
    pub severity: Severity,
    pub action: BreachAction,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RiskReport {
    pub status: RiskStatus,
    pub findings: Vec<RiskFinding>,
    pub evaluated_at: DateTime<Utc>,
}

impl RiskReport {
    pub fn empty(at: DateTime<Utc>) -> Self {
        Self {
            status: RiskStatus::Unknown,
            findings: Vec::new(),
            evaluated_at: at,
        }
    }

    pub fn kill_breaches(&self) -> impl Iterator<Item = &RiskFinding> {
        self.findings
            .iter()
            .filter(|f| f.status == RiskStatus::Breached && f.action == BreachAction::Kill)
    }
}
