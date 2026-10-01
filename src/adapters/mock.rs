use std::sync::Mutex;

use async_trait::async_trait;
use chrono::{DateTime, Utc};

use crate::domain::{
    AuthoritativeSnapshot, BookSide, ConnectionEvent, ConnectionEventKind, ConnectionHealth,
    ConnectionSnapshot, DomainEvent, ExecutionEvent, ExecutionSnapshot, ExecutionVerdict,
    FillRecord, MarketDataEvent, OrderRecord, PositionEvent, PositionRecord,
};
use crate::error::ControlError;

use super::traits::{
    AuthoritativeStateSource, ConnectionSource, ExecutionStateSource, MarketDataSource,
    OrderStateSource, PositionSource, TradeStateSource,
};

/// Deterministic source used by tests and the local demo. No network and no keys.
#[derive(Debug, Clone)]
pub struct MockSources {
    pub fail_rest: bool,
    pub missing_positions: bool,
    pub execution_uncertainty: bool,
    pub reconciliation_mismatch: bool,
    pub snapshot: AuthoritativeSnapshot,
}

impl MockSources {
    pub fn fetch(&self) -> Result<AuthoritativeSnapshot, ControlError> {
        if self.fail_rest {
            return Err(ControlError::Source("injected REST failure".to_string()));
        }
        let mut snapshot = self.snapshot.clone();
        if self.missing_positions {
            snapshot.positions_unknown = true;
            snapshot.positions.clear();
        }
        if self.execution_uncertainty {
            snapshot.execution = ExecutionVerdict::Unknown;
        }
        if self.reconciliation_mismatch {
            for position in &mut snapshot.positions {
                position.size = rust_decimal::Decimal::ZERO;
            }
        }
        Ok(snapshot)
    }
}

#[derive(Debug)]
pub struct MockMarketData {
    pub event: Mutex<MarketDataEvent>,
    pub fail: Mutex<bool>,
}

#[async_trait]
impl MarketDataSource for MockMarketData {
    async fn latest(&self, token_id: &str) -> Result<MarketDataEvent, ControlError> {
        if *self.fail.lock().map_err(|_| ControlError::LockPoisoned)? {
            return Err(ControlError::Source("injected market-data failure".into()));
        }
        let mut event = self
            .event
            .lock()
            .map_err(|_| ControlError::LockPoisoned)?
            .clone();
        event.token_id = token_id.to_string();
        Ok(event)
    }
}

#[derive(Debug)]
pub struct MockBook<T> {
    pub items: Mutex<Vec<T>>,
    pub fail: Mutex<bool>,
}

#[async_trait]
impl OrderStateSource for MockBook<OrderRecord> {
    async fn open_orders(&self) -> Result<Vec<OrderRecord>, ControlError> {
        if *self.fail.lock().map_err(|_| ControlError::LockPoisoned)? {
            return Err(ControlError::Source("injected order-source failure".into()));
        }
        Ok(self
            .items
            .lock()
            .map_err(|_| ControlError::LockPoisoned)?
            .clone())
    }
}

#[async_trait]
impl TradeStateSource for MockBook<FillRecord> {
    async fn recent_trades(&self) -> Result<Vec<FillRecord>, ControlError> {
        if *self.fail.lock().map_err(|_| ControlError::LockPoisoned)? {
            return Err(ControlError::Source("injected trade-source failure".into()));
        }
        Ok(self
            .items
            .lock()
            .map_err(|_| ControlError::LockPoisoned)?
            .clone())
    }
}

#[async_trait]
impl PositionSource for MockBook<PositionRecord> {
    async fn positions(&self) -> Result<Vec<PositionRecord>, ControlError> {
        if *self.fail.lock().map_err(|_| ControlError::LockPoisoned)? {
            return Err(ControlError::Source(
                "injected position-source failure".into(),
            ));
        }
        Ok(self
            .items
            .lock()
            .map_err(|_| ControlError::LockPoisoned)?
            .clone())
    }
}

#[derive(Debug)]
pub struct MockExecution {
    pub snapshot: Mutex<ExecutionSnapshot>,
}

#[async_trait]
impl ExecutionStateSource for MockExecution {
    async fn execution_state(&self) -> Result<ExecutionSnapshot, ControlError> {
        Ok(self
            .snapshot
            .lock()
            .map_err(|_| ControlError::LockPoisoned)?
            .clone())
    }
}

#[derive(Debug)]
pub struct MockConnection {
    pub snapshot: Mutex<ConnectionSnapshot>,
}

#[async_trait]
impl ConnectionSource for MockConnection {
    async fn connection(&self) -> Result<ConnectionSnapshot, ControlError> {
        Ok(self
            .snapshot
            .lock()
            .map_err(|_| ControlError::LockPoisoned)?
            .clone())
    }
}

#[async_trait]
impl AuthoritativeStateSource for MockSources {
    async fn fetch_authoritative(&self) -> Result<AuthoritativeSnapshot, ControlError> {
        self.fetch()
    }
}

pub fn disconnect_event(id: &str, at: DateTime<Utc>) -> DomainEvent {
    DomainEvent::Connection(ConnectionEvent {
        id: id.to_string(),
        kind: ConnectionEventKind::Disconnected {
            error: Some("injected disconnect".to_string()),
        },
        at,
    })
}

pub fn execution_event(verdict: ExecutionVerdict, at: DateTime<Utc>) -> DomainEvent {
    DomainEvent::Execution(ExecutionEvent {
        execution_id: "exec-1".to_string(),
        verdict,
        at,
        detail: verdict.to_string(),
        failed: verdict == ExecutionVerdict::Inconsistent,
    })
}

pub fn unknown_position(at: DateTime<Utc>) -> DomainEvent {
    DomainEvent::Position(PositionEvent {
        side_of_book: BookSide::Remote,
        position: PositionRecord {
            market_id: "unknown".to_string(),
            token_id: "unknown".to_string(),
            size: rust_decimal::Decimal::ZERO,
            avg_price: None,
            updated_at: at,
        },
        unknown: true,
    })
}

pub fn connected_snapshot(id: &str, at: DateTime<Utc>) -> ConnectionSnapshot {
    ConnectionSnapshot {
        id: id.to_string(),
        reported: ConnectionHealth::Connected,
        effective: ConnectionHealth::Connected,
        connected_at: Some(at),
        last_event_at: Some(at),
        reconnect_count: 0,
        last_disconnect_at: None,
        last_error: None,
        updated_at: at,
    }
}
