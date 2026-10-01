use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::{ConnectionHealth, Freshness};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum BookSide {
    Local,
    Remote,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OrderSide {
    Buy,
    Sell,
    /// Present so a future non-exhaustive SDK side is not mislabeled as a buy or sell.
    Unknown,
}

impl std::fmt::Display for OrderSide {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Buy => "BUY",
            Self::Sell => "SELL",
            Self::Unknown => "UNKNOWN",
        })
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum OrderLifecycle {
    Open,
    PartiallyFilled,
    Filled,
    Cancelled,
    Rejected,
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderRecord {
    pub id: String,
    pub market_id: String,
    pub token_id: String,
    pub side: OrderSide,
    pub price: Decimal,
    pub original_size: Decimal,
    pub size_matched: Decimal,
    pub status: OrderLifecycle,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FillRecord {
    pub id: String,
    pub order_id: Option<String>,
    pub market_id: String,
    pub token_id: String,
    pub side: OrderSide,
    pub price: Decimal,
    pub size: Decimal,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionRecord {
    pub market_id: String,
    pub token_id: String,
    pub size: Decimal,
    pub avg_price: Option<Decimal>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExposureSnapshot {
    pub by_market: std::collections::BTreeMap<String, Decimal>,
    pub total: Decimal,
    pub daily_loss: Decimal,
    pub verified: bool,
    pub updated_at: DateTime<Utc>,
}

impl ExposureSnapshot {
    pub fn zero(at: DateTime<Utc>) -> Self {
        Self {
            by_market: std::collections::BTreeMap::new(),
            total: Decimal::ZERO,
            daily_loss: Decimal::ZERO,
            verified: false,
            updated_at: at,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketDataSnapshot {
    pub token_id: String,
    pub market_id: String,
    pub last_event_at: Option<DateTime<Utc>>,
    pub last_snapshot_at: Option<DateTime<Utc>>,
    pub best_bid: Option<Decimal>,
    pub best_ask: Option<Decimal>,
    pub last_trade_price: Option<Decimal>,
    pub freshness: Freshness,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionSnapshot {
    pub id: String,
    /// Last transport signal. May remain `Connected` while `effective` is `Stale`.
    pub reported: ConnectionHealth,
    pub effective: ConnectionHealth,
    pub connected_at: Option<DateTime<Utc>>,
    pub last_event_at: Option<DateTime<Utc>>,
    pub reconnect_count: u32,
    pub last_disconnect_at: Option<DateTime<Utc>>,
    pub last_error: Option<String>,
    pub updated_at: DateTime<Utc>,
}

/// Boundary with the execution verifier. This crate does not re-implement verification.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ExecutionVerdict {
    Verified,
    Unknown,
    Inconsistent,
    NotChecked,
}

impl std::fmt::Display for ExecutionVerdict {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Verified => "VERIFIED",
            Self::Unknown => "UNKNOWN",
            Self::Inconsistent => "INCONSISTENT",
            Self::NotChecked => "NOT_CHECKED",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionSnapshot {
    pub execution_id: Option<String>,
    pub verdict: ExecutionVerdict,
    pub detail: String,
    pub updated_at: DateTime<Utc>,
    pub consecutive_failures: u32,
}

impl ExecutionSnapshot {
    pub fn unchecked(at: DateTime<Utc>) -> Self {
        Self {
            execution_id: None,
            verdict: ExecutionVerdict::NotChecked,
            detail: String::new(),
            updated_at: at,
            consecutive_failures: 0,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KillRecord {
    pub reason: String,
    pub at: DateTime<Utc>,
    pub actor: super::Actor,
}
