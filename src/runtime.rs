use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;
use serde::{Deserialize, Serialize};

use crate::domain::{
    Alert, ConnectionSnapshot, ExecutionSnapshot, ExposureSnapshot, FillRecord, KillRecord,
    MarketDataSnapshot, OrderRecord, PositionRecord, ReconciliationReport, RecoverySession,
    RiskReport, StateTransition, SystemState, TradeDecision, TradingPermission,
};

/// Authoritative in-memory control state. Persisted as one snapshot.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RuntimeState {
    pub system_state: SystemState,
    pub permission: TradingPermission,
    pub decision: TradeDecision,
    pub kill: Option<KillRecord>,
    pub operator_paused: bool,
    /// Set by disconnects, gaps, and failed recovery. A later connect does not clear it.
    pub recovery_required: bool,
    pub orders_local: BTreeMap<String, OrderRecord>,
    pub orders_remote: BTreeMap<String, OrderRecord>,
    pub fills_local: BTreeMap<String, FillRecord>,
    pub fills_remote: BTreeMap<String, FillRecord>,
    pub positions_local: BTreeMap<String, PositionRecord>,
    pub positions_remote: BTreeMap<String, PositionRecord>,
    pub positions_unknown: bool,
    pub exposure: ExposureSnapshot,
    pub daily_loss: Decimal,
    pub risk: RiskReport,
    pub market_data: BTreeMap<String, MarketDataSnapshot>,
    pub connections: BTreeMap<String, ConnectionSnapshot>,
    pub execution: ExecutionSnapshot,
    pub reconciliation: ReconciliationReport,
    pub recovery: Option<RecoverySession>,
    pub alerts: Vec<Alert>,
    pub transitions: Vec<StateTransition>,
    pub persistence_healthy: bool,
    pub updated_at: DateTime<Utc>,
}

impl RuntimeState {
    pub fn initial(now: DateTime<Utc>) -> Self {
        Self {
            system_state: SystemState::Paused,
            permission: TradingPermission::TradingBlocked,
            decision: TradeDecision::Block,
            kill: None,
            operator_paused: false,
            recovery_required: false,
            orders_local: BTreeMap::new(),
            orders_remote: BTreeMap::new(),
            fills_local: BTreeMap::new(),
            fills_remote: BTreeMap::new(),
            positions_local: BTreeMap::new(),
            positions_remote: BTreeMap::new(),
            positions_unknown: true,
            exposure: ExposureSnapshot::zero(now),
            daily_loss: Decimal::ZERO,
            risk: RiskReport::empty(now),
            market_data: BTreeMap::new(),
            connections: BTreeMap::new(),
            execution: ExecutionSnapshot::unchecked(now),
            reconciliation: ReconciliationReport::unknown(now),
            recovery: None,
            alerts: Vec::new(),
            transitions: Vec::new(),
            persistence_healthy: true,
            updated_at: now,
        }
    }
}
