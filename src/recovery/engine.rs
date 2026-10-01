use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use crate::control::ControlPolicy;
use crate::domain::{
    AuthoritativeSnapshot, ExecutionVerdict, FillRecord, OrderRecord, PositionRecord,
    ReconciliationReport, ReconciliationStatus, RecoveryResult, RecoverySession, RecoveryStageName,
    RecoveryTrigger, RiskStatus, StageOutcome,
};
use crate::error::ControlError;
use crate::reconciliation::{mark_recovered, reconcile};
use crate::risk::{self, RiskInput};

/// Outcome of one recovery attempt. The control plane applies it and then
/// transitions. A successful fetch does not by itself clear `recovery_required`.
#[derive(Debug, Clone)]
pub struct RecoveryEffect {
    pub session: RecoverySession,
    pub apply_remote: bool,
    pub orders: BTreeMap<String, OrderRecord>,
    pub fills: BTreeMap<String, FillRecord>,
    pub positions: BTreeMap<String, PositionRecord>,
    pub positions_unknown: bool,
    pub execution: ExecutionVerdict,
    pub execution_id: Option<String>,
    pub reconciliation: ReconciliationReport,
    pub checks_passed: bool,
    pub latch_kill: bool,
}

pub struct RecoveryRequest<'a> {
    pub trigger: RecoveryTrigger,
    pub now: DateTime<Utc>,
    pub policy: &'a ControlPolicy,
    pub orders_local: &'a BTreeMap<String, OrderRecord>,
    pub fills_local: &'a BTreeMap<String, FillRecord>,
    pub positions_local: &'a BTreeMap<String, PositionRecord>,
    pub market_data: &'a BTreeMap<String, crate::domain::MarketDataSnapshot>,
    pub daily_loss: rust_decimal::Decimal,
    pub execution_failures: u32,
    /// Authoritative data can match while the socket is still down.
    /// Recovery does not resume trading until the connection is usable.
    pub connection_ready: bool,
    pub fetch: Result<AuthoritativeSnapshot, ControlError>,
}

pub fn execute(request: RecoveryRequest<'_>) -> RecoveryEffect {
    let mut session = RecoverySession::start(request.trigger, request.now);
    let now = request.now;

    let snapshot = match request.fetch {
        Ok(snapshot) => {
            session.push(
                RecoveryStageName::FetchAuthoritativeState,
                StageOutcome::Succeeded,
                now,
                format!(
                    "orders={} positions={}",
                    snapshot.orders.len(),
                    snapshot.positions.len()
                ),
            );
            snapshot
        }
        Err(err) => {
            session.push(
                RecoveryStageName::FetchAuthoritativeState,
                StageOutcome::Failed,
                now,
                err.to_string(),
            );
            session.finish(RecoveryResult::Failed, now);
            return RecoveryEffect {
                session,
                apply_remote: false,
                orders: BTreeMap::new(),
                fills: BTreeMap::new(),
                positions: BTreeMap::new(),
                positions_unknown: true,
                execution: ExecutionVerdict::Unknown,
                execution_id: None,
                reconciliation: ReconciliationReport::unknown(now),
                checks_passed: false,
                latch_kill: false,
            };
        }
    };

    let orders = map_by(snapshot.orders, |o| o.id.clone());
    let fills = map_by(snapshot.fills, |f| f.id.clone());
    let positions = map_by(snapshot.positions, |p| p.token_id.clone());
    let positions_unknown = snapshot.positions_unknown;
    let execution = snapshot.execution;

    let mut report = reconcile(crate::reconciliation::ReconcileBooks {
        orders_local: request.orders_local,
        orders_remote: &orders,
        fills_local: request.fills_local,
        fills_remote: &fills,
        positions_local: request.positions_local,
        positions_remote: &positions,
        positions_unknown,
        execution,
        now,
    });

    let orders_ok = report.orders == ReconciliationStatus::Match;
    let fills_ok = report.fills == ReconciliationStatus::Match;
    let positions_ok = report.positions == ReconciliationStatus::Match && !positions_unknown;
    session.push(
        RecoveryStageName::ReconcileOrders,
        if orders_ok {
            StageOutcome::Succeeded
        } else {
            StageOutcome::Failed
        },
        now,
        report.orders.to_string(),
    );
    session.push(
        RecoveryStageName::ReconcileFills,
        if fills_ok {
            StageOutcome::Succeeded
        } else {
            StageOutcome::Failed
        },
        now,
        report.fills.to_string(),
    );
    session.push(
        RecoveryStageName::ReconcilePositions,
        if positions_ok {
            StageOutcome::Succeeded
        } else {
            StageOutcome::Failed
        },
        now,
        if positions_unknown {
            "positions unknown".to_string()
        } else {
            report.positions.to_string()
        },
    );

    let exposure = risk::recompute_exposure(
        if positions.is_empty() {
            request.positions_local
        } else {
            &positions
        },
        request.market_data,
        request.daily_loss,
        positions_unknown,
        now,
    );
    session.push(
        RecoveryStageName::RecalculateExposure,
        if exposure.verified {
            StageOutcome::Succeeded
        } else {
            StageOutcome::Failed
        },
        now,
        format!("total={} verified={}", exposure.total, exposure.verified),
    );

    let risk_positions = if positions.is_empty() {
        request.positions_local
    } else {
        &positions
    };
    let risk = risk::evaluate(
        request.policy,
        &RiskInput {
            positions: risk_positions,
            exposure: &exposure,
            market_data: request.market_data,
            daily_loss: request.daily_loss,
            execution_failures: request.execution_failures,
            positions_unknown,
            now,
        },
    );
    let risk_ok = matches!(risk.status, RiskStatus::Healthy | RiskStatus::Warning);
    let latch_kill = risk.kill_breaches().next().is_some();
    session.push(
        RecoveryStageName::RecheckRisk,
        if risk_ok && !latch_kill {
            StageOutcome::Succeeded
        } else {
            StageOutcome::Failed
        },
        now,
        risk.status.to_string(),
    );

    let checks_passed = orders_ok
        && fills_ok
        && positions_ok
        && exposure.verified
        && risk_ok
        && !latch_kill
        && request.connection_ready
        && execution == ExecutionVerdict::Verified;

    if checks_passed {
        mark_recovered(&mut report);
    }

    session.finish(
        if checks_passed {
            RecoveryResult::Completed
        } else {
            RecoveryResult::Failed
        },
        now,
    );

    RecoveryEffect {
        session,
        apply_remote: true,
        orders,
        fills,
        positions,
        positions_unknown,
        execution,
        execution_id: snapshot.execution_id,
        reconciliation: report,
        checks_passed,
        latch_kill,
    }
}

fn map_by<T>(items: Vec<T>, key: impl Fn(&T) -> String) -> BTreeMap<String, T> {
    items.into_iter().map(|item| (key(&item), item)).collect()
}
