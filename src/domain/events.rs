use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use super::{
    ConnectionHealth, ExecutionVerdict, FillRecord, OrderRecord, OrderSide, PositionRecord,
};

/// Normalized internal events. External Polymarket payloads are mapped into these
/// by an adapter. The control plane does not interpret raw exchange JSON.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum DomainEvent {
    Order(OrderEvent),
    Fill(FillEvent),
    Position(PositionEvent),
    MarketData(MarketDataEvent),
    Connection(ConnectionEvent),
    Transaction(TransactionEvent),
    Execution(ExecutionEvent),
    System(SystemEvent),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OrderEvent {
    pub side_of_book: super::BookSide,
    pub order: OrderRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FillEvent {
    pub side_of_book: super::BookSide,
    pub fill: FillRecord,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PositionEvent {
    pub side_of_book: super::BookSide,
    pub position: PositionRecord,
    /// When true, the source could not produce a position and the book is untrusted.
    pub unknown: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MarketDataEvent {
    pub token_id: String,
    pub market_id: String,
    pub at: DateTime<Utc>,
    pub best_bid: Option<Decimal>,
    pub best_ask: Option<Decimal>,
    pub last_trade_price: Option<Decimal>,
    pub snapshot: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ConnectionEvent {
    pub id: String,
    pub kind: ConnectionEventKind,
    pub at: DateTime<Utc>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ConnectionEventKind {
    Connected,
    Disconnected { error: Option<String> },
    Reconnecting { error: Option<String> },
    EventReceived,
    Failed { error: String },
}

impl ConnectionEventKind {
    pub fn reported(&self) -> ConnectionHealth {
        match self {
            Self::Connected | Self::EventReceived => ConnectionHealth::Connected,
            Self::Disconnected { .. } => ConnectionHealth::Disconnected,
            Self::Reconnecting { .. } => ConnectionHealth::Reconnecting,
            Self::Failed { .. } => ConnectionHealth::Failed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TransactionEvent {
    pub execution_id: String,
    pub transaction_id: String,
    pub market_id: String,
    pub token_id: String,
    pub at: DateTime<Utc>,
    pub detail: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionEvent {
    pub execution_id: String,
    pub verdict: ExecutionVerdict,
    pub at: DateTime<Utc>,
    pub detail: String,
    /// Counted toward the execution-failure limit when true.
    pub failed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum SystemEvent {
    EventGap { detail: String },
    SetDailyLoss { loss: Decimal },
    UnexpectedApi { detail: String },
}

/// Authoritative account snapshot used by recovery. Produced by a source trait.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AuthoritativeSnapshot {
    pub orders: Vec<OrderRecord>,
    pub fills: Vec<FillRecord>,
    pub positions: Vec<PositionRecord>,
    pub positions_unknown: bool,
    pub execution: ExecutionVerdict,
    pub execution_id: Option<String>,
    pub fetched_at: DateTime<Utc>,
}

pub fn order_side_label(side: OrderSide) -> &'static str {
    match side {
        OrderSide::Buy => "BUY",
        OrderSide::Sell => "SELL",
        OrderSide::Unknown => "UNKNOWN",
    }
}
