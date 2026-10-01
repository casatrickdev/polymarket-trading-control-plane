use crate::control::ControlPolicy;
use crate::control::StaleDataAction;
use crate::domain::{
    ConnectionHealth, ExecutionVerdict, Freshness, HealthReason, ReconciliationReport,
    ReconciliationStatus, RiskReport, RiskStatus, SystemHealth, SystemState, TradeDecision,
    TradingPermission,
};
use crate::runtime::RuntimeState;

/// Full control assessment: system state, permission, and bot decision.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Assessment {
    pub health: SystemHealth,
    pub permission: TradingPermission,
    pub decision: TradeDecision,
    pub gates_open: bool,
}

pub fn assess(rt: &RuntimeState, policy: &ControlPolicy) -> Assessment {
    let now = rt.updated_at;
    let mut reasons = Vec::new();

    if rt.kill.is_some() {
        reasons.push(HealthReason::KillSwitchActive);
    }
    if rt.recovery.as_ref().is_some_and(|s| s.in_progress()) {
        reasons.push(HealthReason::RecoveryInProgress);
    }
    if rt.recovery_required {
        reasons.push(HealthReason::RecoveryRequired);
    }
    if rt.operator_paused {
        reasons.push(HealthReason::OperatorPaused);
    }
    if !rt.persistence_healthy {
        reasons.push(HealthReason::PersistenceUnhealthy);
    }

    push_connection_reasons(rt, policy, &mut reasons);
    push_market_reasons(rt, policy, &mut reasons);
    push_reconciliation_reasons(&rt.reconciliation, rt.positions_unknown, &mut reasons);
    push_execution_reasons(rt, policy, &mut reasons);
    push_risk_reasons(&rt.risk, &mut reasons);

    if rt.alerts.iter().any(alert_blocks_trading)
        && !reasons
            .iter()
            .any(|r| matches!(r, HealthReason::CriticalAlert))
    {
        reasons.push(HealthReason::CriticalAlert);
    }

    let target = resolve_target(rt, policy, &reasons);
    let permission = resolve_permission(target, policy, &reasons);
    let decision = resolve_decision(target, permission, rt);
    let gates_open = permission == TradingPermission::TradingAllowed;

    Assessment {
        health: SystemHealth {
            state: target,
            reasons,
            evaluated_at: now,
        },
        permission,
        decision,
        gates_open,
    }
}

fn resolve_target(
    rt: &RuntimeState,
    policy: &ControlPolicy,
    reasons: &[HealthReason],
) -> SystemState {
    if rt.kill.is_some() {
        return SystemState::Killed;
    }
    if rt.recovery.as_ref().is_some_and(|s| s.in_progress()) {
        return SystemState::Recovering;
    }
    if !rt.persistence_healthy {
        return SystemState::Failed;
    }
    if rt.operator_paused || rt.recovery_required {
        return SystemState::Paused;
    }
    let blocking = reasons.iter().any(|r| is_blocking(r, policy));
    let warning = reasons.iter().any(|r| is_warning(r, policy));
    if blocking {
        SystemState::Paused
    } else if warning {
        SystemState::Degraded
    } else {
        SystemState::Healthy
    }
}

fn is_blocking(reason: &HealthReason, policy: &ControlPolicy) -> bool {
    match reason {
        HealthReason::MarketDataStale { .. } => policy.stale_data_action == StaleDataAction::Pause,
        HealthReason::RiskWarning => false,
        HealthReason::RecoveryInProgress | HealthReason::KillSwitchActive => false,
        HealthReason::CriticalAlert => true,
        _ => true,
    }
}

fn is_warning(reason: &HealthReason, policy: &ControlPolicy) -> bool {
    match reason {
        HealthReason::RiskWarning => true,
        HealthReason::MarketDataStale { .. } => {
            policy.stale_data_action == StaleDataAction::Degrade
        }
        _ => false,
    }
}

fn resolve_permission(
    target: SystemState,
    policy: &ControlPolicy,
    reasons: &[HealthReason],
) -> TradingPermission {
    let hard = reasons.iter().any(|r| is_hard_block(r, policy));
    let needs_verification = reasons.iter().any(is_verification);
    if target == SystemState::Healthy && !hard && !needs_verification {
        return TradingPermission::TradingAllowed;
    }
    if target == SystemState::Degraded
        && policy.degraded_allows_trading
        && !hard
        && !needs_verification
    {
        return TradingPermission::TradingAllowed;
    }
    if !hard && needs_verification {
        TradingPermission::TradingRequiresVerification
    } else {
        TradingPermission::TradingBlocked
    }
}

fn is_hard_block(reason: &HealthReason, policy: &ControlPolicy) -> bool {
    match reason {
        HealthReason::KillSwitchActive
        | HealthReason::PersistenceUnhealthy
        | HealthReason::OperatorPaused
        | HealthReason::PositionMismatch { .. }
        | HealthReason::ReconciliationMismatch
        | HealthReason::ReconciliationFailed
        | HealthReason::ExecutionInconsistent
        | HealthReason::RiskBreached { .. }
        | HealthReason::RiskUnknown
        | HealthReason::ConnectionMissing
        | HealthReason::ConnectionDisconnected
        | HealthReason::ConnectionReconnecting
        | HealthReason::ConnectionStale
        | HealthReason::ConnectionFailed
        | HealthReason::ConnectedWithoutEvents
        | HealthReason::ReconnectLoop { .. }
        | HealthReason::MarketDataUnknown
        | HealthReason::CriticalAlert => true,
        HealthReason::MarketDataStale { .. } => policy.stale_blocks_trading,
        _ => false,
    }
}

fn is_verification(reason: &HealthReason) -> bool {
    matches!(
        reason,
        HealthReason::ExecutionUnknown
            | HealthReason::ExecutionNotChecked
            | HealthReason::PositionUnknown
            | HealthReason::ReconciliationIncomplete
            | HealthReason::RecoveryRequired
            | HealthReason::RecoveryInProgress
    )
}

fn resolve_decision(
    target: SystemState,
    permission: TradingPermission,
    rt: &RuntimeState,
) -> TradeDecision {
    if permission == TradingPermission::TradingAllowed {
        return TradeDecision::Allow;
    }
    if matches!(target, SystemState::Killed | SystemState::Failed) {
        return TradeDecision::Block;
    }
    if rt.recovery_required || target == SystemState::Recovering {
        return TradeDecision::RequireRecovery;
    }
    if rt.reconciliation.blocks_trading() || rt.positions_unknown {
        return TradeDecision::RequireReconciliation;
    }
    if matches!(
        rt.execution.verdict,
        ExecutionVerdict::Unknown | ExecutionVerdict::Inconsistent | ExecutionVerdict::NotChecked
    ) {
        return TradeDecision::RequireReconciliation;
    }
    TradeDecision::Block
}

fn alert_blocks_trading(alert: &crate::domain::Alert) -> bool {
    if alert.resolved || alert.severity != crate::domain::Severity::Critical {
        return false;
    }
    alert.event_type.is_condition()
        || alert
            .metadata
            .get("external")
            .and_then(|value| value.as_bool())
            == Some(true)
}

fn push_connection_reasons(
    rt: &RuntimeState,
    policy: &ControlPolicy,
    reasons: &mut Vec<HealthReason>,
) {
    if rt.connections.is_empty() {
        reasons.push(HealthReason::ConnectionMissing);
        return;
    }
    let mut loop_count = 0u32;
    let mut saw_connected_quiet = false;
    let mut rank: Option<ConnectionHealth> = None;
    for conn in rt.connections.values() {
        if conn.reconnect_count >= policy.risk.reconnect_loop_threshold {
            loop_count = loop_count.max(conn.reconnect_count);
        }
        if conn.reported == ConnectionHealth::Connected && conn.last_event_at.is_none() {
            saw_connected_quiet = true;
        }
        rank = Some(worse_connection(rank, conn.effective));
    }
    match rank {
        Some(ConnectionHealth::Failed) => reasons.push(HealthReason::ConnectionFailed),
        Some(ConnectionHealth::Disconnected) => reasons.push(HealthReason::ConnectionDisconnected),
        Some(ConnectionHealth::Reconnecting) => reasons.push(HealthReason::ConnectionReconnecting),
        Some(ConnectionHealth::Stale) => {
            if saw_connected_quiet {
                reasons.push(HealthReason::ConnectedWithoutEvents);
            } else {
                reasons.push(HealthReason::ConnectionStale);
            }
        }
        Some(ConnectionHealth::Connected) | None => {}
    }
    if loop_count > 0 {
        reasons.push(HealthReason::ReconnectLoop { count: loop_count });
    }
}

fn worse_connection(current: Option<ConnectionHealth>, next: ConnectionHealth) -> ConnectionHealth {
    fn rank(c: ConnectionHealth) -> u8 {
        match c {
            ConnectionHealth::Connected => 0,
            ConnectionHealth::Stale => 1,
            ConnectionHealth::Reconnecting => 2,
            ConnectionHealth::Disconnected => 3,
            ConnectionHealth::Failed => 4,
        }
    }
    match current {
        Some(cur) if rank(cur) >= rank(next) => cur,
        _ => next,
    }
}

fn push_market_reasons(rt: &RuntimeState, policy: &ControlPolicy, reasons: &mut Vec<HealthReason>) {
    if rt.market_data.is_empty() {
        reasons.push(HealthReason::MarketDataUnknown);
        return;
    }
    let max_age = policy.risk.max_market_data_age_ms;
    for snap in rt.market_data.values() {
        match snap.freshness {
            Freshness::Unknown => reasons.push(HealthReason::MarketDataUnknown),
            Freshness::Stale => {
                let age = snap
                    .last_event_at
                    .or(snap.last_snapshot_at)
                    .map(|ts| crate::domain::age_ms(rt.updated_at, ts))
                    .unwrap_or(max_age.saturating_add(1));
                reasons.push(HealthReason::MarketDataStale {
                    token_id: snap.token_id.clone(),
                    age_ms: age,
                    max_age_ms: max_age,
                });
            }
            Freshness::Fresh => {}
        }
    }
}

fn push_reconciliation_reasons(
    report: &ReconciliationReport,
    positions_unknown: bool,
    reasons: &mut Vec<HealthReason>,
) {
    if positions_unknown {
        reasons.push(HealthReason::PositionUnknown);
    }
    match report.positions {
        ReconciliationStatus::Mismatch => {
            if let Some(m) = report
                .mismatches
                .iter()
                .find(|m| m.entity_type == crate::domain::EntityType::Position)
            {
                reasons.push(HealthReason::PositionMismatch {
                    token_id: m.entity_id.clone(),
                });
            } else {
                reasons.push(HealthReason::ReconciliationMismatch);
            }
        }
        ReconciliationStatus::Unknown => reasons.push(HealthReason::ReconciliationIncomplete),
        ReconciliationStatus::Failed => reasons.push(HealthReason::ReconciliationFailed),
        ReconciliationStatus::Match | ReconciliationStatus::Recovered => {}
    }
    let book_mismatch = matches!(
        report.orders,
        ReconciliationStatus::Mismatch | ReconciliationStatus::Failed
    ) || matches!(
        report.fills,
        ReconciliationStatus::Mismatch | ReconciliationStatus::Failed
    ) || matches!(
        report.execution,
        ReconciliationStatus::Mismatch | ReconciliationStatus::Failed
    );
    let already_reported = reasons.iter().any(|r| {
        matches!(
            r,
            HealthReason::ReconciliationMismatch | HealthReason::PositionMismatch { .. }
        )
    });
    if book_mismatch && !already_reported {
        reasons.push(HealthReason::ReconciliationMismatch);
    }
    if report.execution == ReconciliationStatus::Unknown && !positions_unknown {
        reasons.push(HealthReason::ReconciliationIncomplete);
    }
}

fn push_execution_reasons(
    rt: &RuntimeState,
    policy: &ControlPolicy,
    reasons: &mut Vec<HealthReason>,
) {
    match rt.execution.verdict {
        ExecutionVerdict::Verified => {}
        ExecutionVerdict::Unknown => reasons.push(HealthReason::ExecutionUnknown),
        ExecutionVerdict::Inconsistent => reasons.push(HealthReason::ExecutionInconsistent),
        ExecutionVerdict::NotChecked if policy.require_execution_verification => {
            reasons.push(HealthReason::ExecutionNotChecked);
        }
        ExecutionVerdict::NotChecked => {}
    }
}

fn push_risk_reasons(risk: &RiskReport, reasons: &mut Vec<HealthReason>) {
    match risk.status {
        RiskStatus::Breached => {
            let rule = risk
                .findings
                .iter()
                .find(|f| f.status == RiskStatus::Breached)
                .map(|f| f.rule.as_str().to_string())
                .unwrap_or_else(|| "UNKNOWN".to_string());
            reasons.push(HealthReason::RiskBreached { rule });
        }
        RiskStatus::Unknown => reasons.push(HealthReason::RiskUnknown),
        RiskStatus::Warning => reasons.push(HealthReason::RiskWarning),
        RiskStatus::Healthy => {}
    }
}
