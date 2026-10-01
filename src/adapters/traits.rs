use async_trait::async_trait;

use crate::domain::{
    AuthoritativeSnapshot, ConnectionSnapshot, ExecutionSnapshot, FillRecord, MarketDataEvent,
    OrderRecord, PositionRecord,
};
use crate::error::ControlError;

/// Read-only boundaries. The control plane does not place orders.
#[async_trait]
pub trait MarketDataSource: Send + Sync {
    async fn latest(&self, token_id: &str) -> Result<MarketDataEvent, ControlError>;
}

#[async_trait]
pub trait OrderStateSource: Send + Sync {
    async fn open_orders(&self) -> Result<Vec<OrderRecord>, ControlError>;
}

#[async_trait]
pub trait TradeStateSource: Send + Sync {
    async fn recent_trades(&self) -> Result<Vec<FillRecord>, ControlError>;
}

#[async_trait]
pub trait PositionSource: Send + Sync {
    async fn positions(&self) -> Result<Vec<PositionRecord>, ControlError>;
}

#[async_trait]
pub trait ExecutionStateSource: Send + Sync {
    async fn execution_state(&self) -> Result<ExecutionSnapshot, ControlError>;
}

#[async_trait]
pub trait ConnectionSource: Send + Sync {
    async fn connection(&self) -> Result<ConnectionSnapshot, ControlError>;
}

#[async_trait]
pub trait AuthoritativeStateSource: Send + Sync {
    async fn fetch_authoritative(&self) -> Result<AuthoritativeSnapshot, ControlError>;
}
