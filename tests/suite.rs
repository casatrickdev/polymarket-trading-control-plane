use chrono::Duration;
use polymarket_trading_control_plane::adapters::MockSources;
use polymarket_trading_control_plane::demo::{
    self, apply_healthy, authoritative, connection, position, t0, CONNECTION, TOKEN,
};
use polymarket_trading_control_plane::domain::{
    Alert, AlertType, BookSide, ConnectionEventKind, DomainEvent, ExecutionEvent, ExecutionVerdict,
    RecoveryTrigger, Severity, SystemEvent, SystemState, TradeDecision, TradingPermission,
};
use polymarket_trading_control_plane::repository::{FailingStore, MemoryStore};
use polymarket_trading_control_plane::{ControlPlane, ControlPolicy};
use rust_decimal::Decimal;
use serde_json::json;

fn plane() -> ControlPlane<MemoryStore> {
    demo::memory_plane(t0())
}

fn healthy() -> ControlPlane<MemoryStore> {
    let mut plane = plane();
    apply_healthy(&mut plane).unwrap();
    plane
}

#[test]
fn healthy_state_allows_trading() {
    let plane = healthy();
    let gate = plane.trade_gate();
    assert_eq!(gate.system_state, SystemState::Healthy);
    assert_eq!(gate.permission, TradingPermission::TradingAllowed);
    assert_eq!(gate.decision, TradeDecision::Allow);
    assert!(gate.reasons.is_empty());
}

#[test]
fn degraded_when_position_nears_limit() {
    let mut plane = healthy();
    plane.ingest(position(BookSide::Local, 450, t0())).unwrap();
    plane.ingest(position(BookSide::Remote, 450, t0())).unwrap();
    assert_eq!(plane.runtime().system_state, SystemState::Degraded);
    assert_eq!(
        plane.runtime().permission,
        TradingPermission::TradingBlocked
    );
    assert!(
        plane.runtime().risk.status
            == polymarket_trading_control_plane::domain::RiskStatus::Warning
    );
}

#[test]
fn paused_on_operator_pause() {
    let mut plane = healthy();
    plane.pause("operator hold".into()).unwrap();
    assert_eq!(plane.runtime().system_state, SystemState::Paused);
    assert_eq!(
        plane.runtime().permission,
        TradingPermission::TradingBlocked
    );
}

#[test]
fn recovering_blocks_until_checks_pass() {
    let mut plane = healthy();
    plane
        .ingest(connection(
            ConnectionEventKind::Disconnected { error: None },
            t0(),
        ))
        .unwrap();
    assert_eq!(plane.runtime().system_state, SystemState::Paused);
    plane
        .ingest(connection(
            ConnectionEventKind::Reconnecting { error: None },
            t0(),
        ))
        .unwrap();
    assert_ne!(plane.runtime().system_state, SystemState::Healthy);
    let while_reconnecting = plane
        .recover_with(
            RecoveryTrigger::WebsocketReconnect,
            Ok(authoritative(100, ExecutionVerdict::Verified, t0())),
        )
        .unwrap();
    assert_eq!(
        while_reconnecting.evaluation.system_state,
        SystemState::Paused
    );
    plane
        .ingest(connection(ConnectionEventKind::Connected, t0()))
        .unwrap();
    plane
        .ingest(connection(ConnectionEventKind::EventReceived, t0()))
        .unwrap();
    assert_ne!(plane.runtime().system_state, SystemState::Healthy);
    let outcome = plane
        .recover_with(
            RecoveryTrigger::WebsocketReconnect,
            Ok(authoritative(100, ExecutionVerdict::Verified, t0())),
        )
        .unwrap();
    assert_eq!(outcome.evaluation.system_state, SystemState::Healthy);
    assert!(plane.runtime().recovery.as_ref().unwrap().stages.len() >= 6);
}

#[test]
fn failed_on_persistence_loss() {
    let store = FailingStore::new(MemoryStore::new());
    let mut plane = ControlPlane::new(store, ControlPolicy::default(), t0()).unwrap();
    apply_healthy(&mut plane).unwrap();
    plane.store().set_fail_writes(true);
    let err = plane
        .ingest(position(BookSide::Local, 100, t0()))
        .unwrap_err();
    assert!(err.to_string().contains("persistence"));
    assert_ne!(
        plane.trade_gate().permission,
        TradingPermission::TradingAllowed
    );
    assert_eq!(plane.runtime().system_state, SystemState::Failed);
}

#[test]
fn stale_market_data_blocks_and_degrade_policy_differs() {
    let paused = demo::scenario_stale().unwrap();
    assert_eq!(paused.system_state, SystemState::Paused);
    assert_eq!(paused.permission, TradingPermission::TradingBlocked);
    let degraded = demo::scenario_stale_degraded().unwrap();
    assert_eq!(degraded.system_state, SystemState::Degraded);
    assert_eq!(degraded.permission, TradingPermission::TradingBlocked);
}

#[test]
fn position_mismatch_requires_reconciliation() {
    let report = demo::scenario_position_mismatch().unwrap();
    assert_eq!(report.system_state, SystemState::Paused);
    assert_eq!(report.decision, TradeDecision::RequireReconciliation);
    assert!(report.detail.contains("difference=-60"));
}

#[test]
fn unresolved_execution_blocks_trading() {
    let mut plane = healthy();
    plane
        .ingest(DomainEvent::Execution(ExecutionEvent {
            execution_id: "exec-2".into(),
            verdict: ExecutionVerdict::Unknown,
            at: t0(),
            detail: "verifier unknown".into(),
            failed: false,
        }))
        .unwrap();
    assert_ne!(
        plane.trade_gate().permission,
        TradingPermission::TradingAllowed
    );
    assert_eq!(
        plane.trade_gate().decision,
        TradeDecision::RequireReconciliation
    );
}

#[test]
fn risk_rules() {
    let mut position_breach = healthy();
    position_breach
        .ingest(position(BookSide::Local, 600, t0()))
        .unwrap();
    position_breach
        .ingest(position(BookSide::Remote, 600, t0()))
        .unwrap();
    assert!(has_rule(&position_breach, "MAX_POSITION"));

    let mut market = healthy();
    let mut policy = ControlPolicy::default();
    policy.risk.max_market_exposure = Decimal::from(10);
    market.set_policy(policy).unwrap();
    assert!(has_rule(&market, "MAX_MARKET_EXPOSURE"));

    let mut total = healthy();
    let mut policy = ControlPolicy::default();
    policy.risk.max_total_exposure = Decimal::from(10);
    total.set_policy(policy).unwrap();
    assert!(has_rule(&total, "MAX_TOTAL_EXPOSURE"));

    let daily = demo::scenario_daily_loss_kill().unwrap();
    assert_eq!(daily.system_state, SystemState::Killed);

    let mut failures = healthy();
    for index in 0..6 {
        failures
            .ingest(DomainEvent::Execution(ExecutionEvent {
                execution_id: format!("fail-{index}"),
                verdict: ExecutionVerdict::Verified,
                at: t0(),
                detail: "execution failed".into(),
                failed: true,
            }))
            .unwrap();
    }
    assert!(has_rule(&failures, "MAX_EXECUTION_FAILURES"));

    let stale = demo::scenario_stale().unwrap();
    assert_eq!(stale.system_state, SystemState::Paused);
}

#[test]
fn reconciliation_outcomes() {
    let matched = healthy();
    assert_eq!(
        matched.runtime().reconciliation.positions,
        polymarket_trading_control_plane::domain::ReconciliationStatus::Match
    );

    let mut mismatch = healthy();
    mismatch
        .ingest(position(BookSide::Remote, 40, t0()))
        .unwrap();
    let found = mismatch
        .runtime()
        .reconciliation
        .mismatches
        .iter()
        .find(|item| item.entity_id == TOKEN)
        .unwrap();
    assert_eq!(found.expected, Decimal::from(100));
    assert_eq!(found.observed, Decimal::from(40));
    assert_eq!(found.difference, Decimal::from(-60));

    let mut unknown = healthy();
    unknown
        .ingest(DomainEvent::Position(
            polymarket_trading_control_plane::domain::PositionEvent {
                side_of_book: BookSide::Remote,
                position: position_record_ignored(),
                unknown: true,
            },
        ))
        .unwrap();
    assert_eq!(
        unknown.runtime().reconciliation.status,
        polymarket_trading_control_plane::domain::ReconciliationStatus::Unknown
    );
    assert_ne!(
        unknown.trade_gate().permission,
        TradingPermission::TradingAllowed
    );

    let mut execution = healthy();
    execution
        .ingest(DomainEvent::Execution(ExecutionEvent {
            execution_id: "exec-x".into(),
            verdict: ExecutionVerdict::Inconsistent,
            at: t0(),
            detail: "inconsistent".into(),
            failed: true,
        }))
        .unwrap();
    assert_eq!(
        execution.runtime().reconciliation.execution,
        polymarket_trading_control_plane::domain::ReconciliationStatus::Mismatch
    );

    let mut recovered = healthy();
    recovered
        .ingest(connection(
            ConnectionEventKind::Disconnected { error: None },
            t0(),
        ))
        .unwrap();
    recovered
        .ingest(connection(ConnectionEventKind::Connected, t0()))
        .unwrap();
    recovered
        .ingest(connection(ConnectionEventKind::EventReceived, t0()))
        .unwrap();
    assert_ne!(recovered.runtime().system_state, SystemState::Healthy);
    recovered
        .recover_with(
            RecoveryTrigger::StateMismatch,
            Ok(authoritative(100, ExecutionVerdict::Verified, t0())),
        )
        .unwrap();
    assert_eq!(recovered.runtime().system_state, SystemState::Healthy);
    assert_eq!(
        recovered.runtime().recovery.as_ref().unwrap().result,
        Some(polymarket_trading_control_plane::domain::RecoveryResult::Completed)
    );

    let mut failed = healthy();
    failed
        .ingest(connection(
            ConnectionEventKind::Disconnected { error: None },
            t0(),
        ))
        .unwrap();
    let mut source = MockSources {
        fail_rest: true,
        missing_positions: false,
        execution_uncertainty: false,
        reconciliation_mismatch: false,
        snapshot: authoritative(100, ExecutionVerdict::Verified, t0()),
    };
    failed
        .recover_with(RecoveryTrigger::UnexpectedApiResponse, source.fetch())
        .unwrap();
    assert_eq!(failed.runtime().system_state, SystemState::Paused);
    assert_ne!(
        failed.trade_gate().permission,
        TradingPermission::TradingAllowed
    );
    source.fail_rest = false;
    source.missing_positions = true;
    failed
        .recover_with(RecoveryTrigger::Operator, source.fetch())
        .unwrap();
    assert_eq!(failed.runtime().system_state, SystemState::Paused);
}

#[test]
fn websocket_states() {
    let mut plane = plane();
    plane
        .ingest(connection(ConnectionEventKind::Connected, t0()))
        .unwrap();
    plane
        .ingest(connection(ConnectionEventKind::EventReceived, t0()))
        .unwrap();
    assert_eq!(
        plane.runtime().connections[CONNECTION].effective,
        polymarket_trading_control_plane::domain::ConnectionHealth::Connected
    );

    plane
        .ingest(connection(
            ConnectionEventKind::Disconnected {
                error: Some("bye".into()),
            },
            t0(),
        ))
        .unwrap();
    assert_eq!(
        plane.runtime().connections[CONNECTION].reported,
        polymarket_trading_control_plane::domain::ConnectionHealth::Disconnected
    );
    assert_eq!(plane.runtime().system_state, SystemState::Paused);

    plane
        .ingest(connection(
            ConnectionEventKind::Reconnecting {
                error: Some("retry".into()),
            },
            t0(),
        ))
        .unwrap();
    assert_eq!(
        plane.runtime().connections[CONNECTION].reported,
        polymarket_trading_control_plane::domain::ConnectionHealth::Reconnecting
    );
    assert_ne!(plane.runtime().system_state, SystemState::Healthy);

    let mut fresh = healthy();
    let mut policy = ControlPolicy::default();
    policy.risk.max_connection_event_age_ms = 1_000;
    policy.risk.max_market_data_age_ms = 60_000;
    fresh.set_policy(policy).unwrap();
    fresh.set_now(t0() + Duration::seconds(3)).unwrap();
    assert_eq!(
        fresh.runtime().connections[CONNECTION].effective,
        polymarket_trading_control_plane::domain::ConnectionHealth::Stale
    );
    assert_ne!(
        fresh.trade_gate().permission,
        TradingPermission::TradingAllowed
    );

    let mut looping = healthy();
    for _ in 0..5 {
        looping
            .ingest(connection(
                ConnectionEventKind::Disconnected { error: None },
                t0(),
            ))
            .unwrap();
        looping
            .ingest(connection(ConnectionEventKind::Connected, t0()))
            .unwrap();
        looping
            .ingest(connection(ConnectionEventKind::EventReceived, t0()))
            .unwrap();
    }
    assert!(looping.runtime().connections[CONNECTION].reconnect_count >= 5);
    assert!(looping.trade_gate().reasons.iter().any(|reason| matches!(
        reason,
        polymarket_trading_control_plane::domain::HealthReason::ReconnectLoop { .. }
    )));
    assert_ne!(
        looping.trade_gate().permission,
        TradingPermission::TradingAllowed
    );
}

#[test]
fn recovery_does_not_resume_on_reconnect_alone() {
    let report = demo::scenario_disconnect_recovery().unwrap();
    assert!(report.detail.contains("after_disconnect=PAUSED"));
    assert!(report.detail.contains("after_reconnect=PAUSED"));
    assert_eq!(report.system_state, SystemState::Healthy);
    assert_eq!(report.permission, TradingPermission::TradingAllowed);
}

#[test]
fn recovery_failure_remains_paused() {
    let mut plane = healthy();
    plane
        .ingest(DomainEvent::System(SystemEvent::EventGap {
            detail: "gap".into(),
        }))
        .unwrap();
    plane
        .recover_with(
            RecoveryTrigger::SuspectedEventGap,
            Ok(authoritative(0, ExecutionVerdict::Unknown, t0())),
        )
        .unwrap();
    assert_eq!(plane.runtime().system_state, SystemState::Paused);
    assert!(plane.runtime().recovery_required);
    assert_ne!(plane.trade_gate().decision, TradeDecision::Allow);
}

#[test]
fn alerts_dedup_resolve_and_critical_block() {
    let mut plane = healthy();
    let mut policy = ControlPolicy::default();
    policy.risk.max_market_data_age_ms = 1_000;
    policy.risk.max_connection_event_age_ms = 60_000;
    plane.set_policy(policy).unwrap();
    plane.set_now(t0() + Duration::seconds(3)).unwrap();
    let stale_alerts = plane
        .runtime()
        .alerts
        .iter()
        .filter(|alert| alert.event_type == AlertType::MarketDataStale && !alert.resolved)
        .count();
    assert_eq!(stale_alerts, 1);
    plane.set_now(t0() + Duration::seconds(9)).unwrap();
    let stale_alerts = plane
        .runtime()
        .alerts
        .iter()
        .filter(|alert| alert.event_type == AlertType::MarketDataStale && !alert.resolved)
        .count();
    assert_eq!(stale_alerts, 1);
    plane.set_now(t0()).unwrap();
    assert!(plane
        .runtime()
        .alerts
        .iter()
        .filter(|alert| alert.event_type == AlertType::MarketDataStale)
        .all(|alert| alert.resolved));

    let mut blocked = healthy();
    blocked
        .raise_external_alert(Alert {
            id: "ext-1".into(),
            event_type: AlertType::OrderFailure,
            severity: Severity::Critical,
            at: t0(),
            scope: "order".into(),
            entity_id: Some("order-1".into()),
            message: "external critical".into(),
            metadata: json!({"external": true}),
            resolved: false,
            resolved_at: None,
            dedup_key: String::new(),
        })
        .unwrap();
    assert_ne!(
        blocked.trade_gate().permission,
        TradingPermission::TradingAllowed
    );

    blocked
        .raise_external_alert(Alert {
            id: "ext-2".into(),
            event_type: AlertType::OrderFailure,
            severity: Severity::Critical,
            at: t0(),
            scope: "order".into(),
            entity_id: Some("order-1".into()),
            message: "duplicate".into(),
            metadata: json!({"external": true}),
            resolved: false,
            resolved_at: None,
            dedup_key: String::new(),
        })
        .unwrap();
    assert_eq!(
        blocked
            .runtime()
            .alerts
            .iter()
            .filter(|alert| alert.event_type == AlertType::OrderFailure)
            .count(),
        1
    );
}

#[test]
fn commands_are_guarded_and_idempotent() {
    let mut plane = healthy();
    plane.pause("once".into()).unwrap();
    let transitions = plane.runtime().transitions.len();
    let second = plane.pause("twice".into()).unwrap();
    assert!(second.idempotent);
    assert_eq!(plane.runtime().transitions.len(), transitions);
    assert_eq!(plane.runtime().system_state, SystemState::Paused);

    let resume = plane.resume().unwrap();
    assert_eq!(resume.evaluation.system_state, SystemState::Healthy);

    let mut stuck = healthy();
    stuck.ingest(position(BookSide::Remote, 1, t0())).unwrap();
    assert!(stuck.resume().is_err());
    assert_eq!(stuck.runtime().system_state, SystemState::Paused);

    let mut killed = healthy();
    killed.kill("stop".into()).unwrap();
    assert!(killed.kill("stop again".into()).unwrap().idempotent);
    assert!(killed.resume().is_err());
    assert_eq!(killed.runtime().system_state, SystemState::Killed);
    assert!(killed
        .recover_with(
            RecoveryTrigger::Operator,
            Ok(authoritative(100, ExecutionVerdict::Verified, t0()))
        )
        .is_err());

    let mut recon = healthy();
    recon.ingest(position(BookSide::Remote, 40, t0())).unwrap();
    let first = recon.reconcile_command().unwrap();
    let alerts = recon.runtime().alerts.len();
    let second = recon.reconcile_command().unwrap();
    assert!(second.idempotent || recon.runtime().alerts.len() == alerts);
    assert_eq!(
        first.evaluation.decision,
        TradeDecision::RequireReconciliation
    );
    assert_eq!(second.evaluation.system_state, SystemState::Paused);

    let mut recover = healthy();
    let again = recover
        .recover_with(
            RecoveryTrigger::Operator,
            Err(polymarket_trading_control_plane::ControlError::Source(
                "unused".into(),
            )),
        )
        .unwrap();
    assert!(again.idempotent);
    assert_eq!(recover.runtime().system_state, SystemState::Healthy);
}

#[test]
fn fail_closed_on_injected_faults() {
    let mut plane = healthy();
    plane
        .ingest(connection(
            ConnectionEventKind::Disconnected {
                error: Some("injected".into()),
            },
            t0(),
        ))
        .unwrap();
    assert_ne!(plane.trade_gate().decision, TradeDecision::Allow);

    let mut source = MockSources {
        fail_rest: false,
        missing_positions: true,
        execution_uncertainty: true,
        reconciliation_mismatch: true,
        snapshot: authoritative(100, ExecutionVerdict::Verified, t0()),
    };
    plane
        .recover_with(RecoveryTrigger::WebsocketReconnect, source.fetch())
        .unwrap();
    assert_ne!(
        plane.trade_gate().permission,
        TradingPermission::TradingAllowed
    );

    source.missing_positions = false;
    source.execution_uncertainty = false;
    source.reconciliation_mismatch = false;
    source.fail_rest = true;
    plane
        .recover_with(RecoveryTrigger::Operator, source.fetch())
        .unwrap();
    assert_eq!(plane.runtime().system_state, SystemState::Paused);
}

fn has_rule(plane: &ControlPlane<MemoryStore>, rule: &str) -> bool {
    plane.runtime().risk.findings.iter().any(|finding| {
        finding.rule.as_str() == rule
            && finding.status == polymarket_trading_control_plane::domain::RiskStatus::Breached
    })
}

fn position_record_ignored() -> polymarket_trading_control_plane::domain::PositionRecord {
    polymarket_trading_control_plane::domain::PositionRecord {
        market_id: demo::MARKET.into(),
        token_id: TOKEN.into(),
        size: Decimal::ZERO,
        avg_price: None,
        updated_at: t0(),
    }
}
