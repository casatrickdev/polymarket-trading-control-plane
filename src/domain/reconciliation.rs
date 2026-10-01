use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::Severity;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReconciliationStatus {
    Match,
    Mismatch,
    Unknown,
    Recovered,
    Failed,
}

impl std::fmt::Display for ReconciliationStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Match => "MATCH",
            Self::Mismatch => "MISMATCH",
            Self::Unknown => "UNKNOWN",
            Self::Recovered => "RECOVERED",
            Self::Failed => "FAILED",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum EntityType {
    Order,
    Fill,
    Position,
    Exposure,
    Execution,
}

impl std::fmt::Display for EntityType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Order => "ORDER",
            Self::Fill => "FILL",
            Self::Position => "POSITION",
            Self::Exposure => "EXPOSURE",
            Self::Execution => "EXECUTION",
        })
    }
}

/// Numeric or status disagreement between local and authoritative state.
///
/// For positions, `expected` is local, `observed` is remote, and
/// `difference` is observed minus expected.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Mismatch {
    pub entity_type: EntityType,
    pub entity_id: String,
    pub expected: Decimal,
    pub observed: Decimal,
    pub difference: Decimal,
    pub at: DateTime<Utc>,
    pub source: String,
    pub reason: String,
    pub severity: Severity,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReconciliationReport {
    pub status: ReconciliationStatus,
    pub mismatches: Vec<Mismatch>,
    pub orders: ReconciliationStatus,
    pub fills: ReconciliationStatus,
    pub positions: ReconciliationStatus,
    pub execution: ReconciliationStatus,
    pub at: DateTime<Utc>,
}

impl ReconciliationReport {
    pub fn unknown(at: DateTime<Utc>) -> Self {
        Self {
            status: ReconciliationStatus::Unknown,
            mismatches: Vec::new(),
            orders: ReconciliationStatus::Unknown,
            fills: ReconciliationStatus::Unknown,
            positions: ReconciliationStatus::Unknown,
            execution: ReconciliationStatus::Unknown,
            at,
        }
    }

    pub fn blocks_trading(&self) -> bool {
        !matches!(
            self.status,
            ReconciliationStatus::Match | ReconciliationStatus::Recovered
        )
    }
}
