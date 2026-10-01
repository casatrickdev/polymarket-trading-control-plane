//! Credential-free scenarios. The example and the test suite both call these.

use chrono::{DateTime, TimeZone, Utc};
use rust_decimal::Decimal;

use crate::control::{ControlPlane, ControlPolicy, StaleDataAction};
use crate::domain::{
    AuthoritativeSnapshot, BookSide, ConnectionEvent, ConnectionEventKind, DomainEvent,
    ExecutionEvent, ExecutionVerdict, MarketDataEvent, PositionEvent, PositionRecord,
    RecoveryTrigger, SystemEvent, SystemState, TradeDecision, TradingPermission,
};
use crate::error::ControlError;
use crate::repository::{MemoryStore, StateStore};

pub const MARKET: &str = "market-btc";
pub const TOKEN: &str = "token-yes";
pub const CONNECTION: &str = "polymarket-clob";

pub fn t0() -> DateTime<Utc> {
    Utc.with_ymd_and_hms(2026, 10, 1, 12, 0, 0).unwrap()
}

pub fn memory_plane(now: DateTime<Utc>) -> ControlPlane<MemoryStore> {
    ControlPlane::new(MemoryStore::new(), ControlPolicy::default(), now).expect("memory plane")
}

pub fn apply_healthy<S: StateStore>(plane: &mut ControlPlane<S>) -> Result<(), ControlError> {
    let now = t0();
    plane.ingest(connection(ConnectionEventKind::Connected, now))?;
    plane.ingest(connection(ConnectionEventKind::EventReceived, now))?;
    plane.ingest(DomainEvent::MarketData(MarketDataEvent {
        token_id: TOKEN.to_string(),
        market_id: MARKET.to_string(),
        at: now,
        best_bid: Some(Decimal::new(39, 2)),
        best_ask: Some(Decimal::new(41, 2)),
        last_trade_price: Some(Decimal::new(40, 2)),
        snapshot: true,
    }))?;
    plane.ingest(position(BookSide::Local, 100, now))?;
    plane.ingest(position(BookSide::Remote, 100, now))?;
    plane.ingest(DomainEvent::Execution(ExecutionEvent {
        execution_id: "exec-1".to_string(),
        verdict: ExecutionVerdict::Verified,
        at: now,
        detail: "verifier verified".to_string(),
        failed: false,
    }))?;
    Ok(())
}

pub fn position(side: BookSide, size: i64, at: DateTime<Utc>) -> DomainEvent {
    DomainEvent::Position(PositionEvent {
        side_of_book: side,
        unknown: false,
        position: PositionRecord {
            market_id: MARKET.to_string(),
            token_id: TOKEN.to_string(),
            size: Decimal::from(size),
            avg_price: Some(Decimal::new(40, 2)),
            updated_at: at,
        },
    })
}

pub fn connection(kind: ConnectionEventKind, at: DateTime<Utc>) -> DomainEvent {
    DomainEvent::Connection(ConnectionEvent {
        id: CONNECTION.to_string(),
        kind,
        at,
    })
}

pub fn authoritative(
    size: i64,
    execution: ExecutionVerdict,
    at: DateTime<Utc>,
) -> AuthoritativeSnapshot {
    AuthoritativeSnapshot {
        orders: Vec::new(),
        fills: Vec::new(),
        positions: vec![PositionRecord {
            market_id: MARKET.to_string(),
            token_id: TOKEN.to_string(),
            size: Decimal::from(size),
            avg_price: Some(Decimal::new(40, 2)),
            updated_at: at,
        }],
        positions_unknown: false,
        execution,
        execution_id: Some("exec-1".to_string()),
        fetched_at: at,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScenarioReport {
    pub name: &'static str,
    pub system_state: SystemState,
    pub permission: TradingPermission,
    pub decision: TradeDecision,
    pub detail: String,
}

pub fn run_all() -> Result<Vec<ScenarioReport>, ControlError> {
    Ok(vec![
        scenario_healthy()?,
        scenario_stale()?,
        scenario_stale_degraded()?,
        scenario_position_mismatch()?,
        scenario_disconnect_recovery()?,
        scenario_risk_breach()?,
        scenario_kill()?,
    ])
}

pub fn scenario_healthy() -> Result<ScenarioReport, ControlError> {
    let mut plane = memory_plane(t0());
    apply_healthy(&mut plane)?;
    Ok(report(
        "healthy",
        &plane,
        "fresh data, matched positions, verified execution",
    ))
}

pub fn scenario_stale() -> Result<ScenarioReport, ControlError> {
    let mut plane = memory_plane(t0());
    let mut policy = ControlPolicy::default();
    policy.risk.max_market_data_age_ms = 2_000;
    policy.risk.max_connection_event_age_ms = 60_000;
    policy.stale_data_action = StaleDataAction::Pause;
    plane.set_policy(policy)?;
    apply_healthy(&mut plane)?;
    plane.set_now(t0() + chrono::Duration::seconds(4))?;
    Ok(report(
        "stale market data (pause policy)",
        &plane,
        "age 4s, max age 2s, action PAUSE",
    ))
}

pub fn scenario_stale_degraded() -> Result<ScenarioReport, ControlError> {
    let mut plane = memory_plane(t0());
    let mut policy = ControlPolicy::default();
    policy.risk.max_market_data_age_ms = 2_000;
    policy.risk.max_connection_event_age_ms = 60_000;
    policy.stale_data_action = StaleDataAction::Degrade;
    plane.set_policy(policy)?;
    apply_healthy(&mut plane)?;
    plane.set_now(t0() + chrono::Duration::seconds(4))?;
    Ok(report(
        "stale market data (degrade policy)",
        &plane,
        "age 4s, max age 2s, action DEGRADE",
    ))
}

pub fn scenario_position_mismatch() -> Result<ScenarioReport, ControlError> {
    let mut plane = memory_plane(t0());
    apply_healthy(&mut plane)?;
    plane.ingest(position(BookSide::Remote, 40, t0()))?;
    let mismatch = plane
        .runtime()
        .reconciliation
        .mismatches
        .iter()
        .find(|item| item.entity_id == TOKEN)
        .map(|item| {
            format!(
                "expected={} observed={} difference={}",
                item.expected, item.observed, item.difference
            )
        })
        .unwrap_or_else(|| "mismatch missing".to_string());
    Ok(report("position mismatch", &plane, &mismatch))
}

pub fn scenario_disconnect_recovery() -> Result<ScenarioReport, ControlError> {
    let mut plane = memory_plane(t0());
    apply_healthy(&mut plane)?;
    plane.ingest(connection(
        ConnectionEventKind::Disconnected {
            error: Some("socket closed".to_string()),
        },
        t0(),
    ))?;
    let after_drop = plane.runtime().system_state;
    plane.ingest(connection(ConnectionEventKind::Connected, t0()))?;
    plane.ingest(connection(ConnectionEventKind::EventReceived, t0()))?;
    let after_reconnect = plane.runtime().system_state;
    plane.recover_with(
        RecoveryTrigger::WebsocketReconnect,
        Ok(authoritative(100, ExecutionVerdict::Verified, t0())),
    )?;
    let mut detail = format!("after_disconnect={after_drop} after_reconnect={after_reconnect}");
    detail.push_str(&format!(" final={}", plane.runtime().system_state));
    Ok(report("websocket disconnect and recovery", &plane, &detail))
}

pub fn scenario_risk_breach() -> Result<ScenarioReport, ControlError> {
    let mut plane = memory_plane(t0());
    apply_healthy(&mut plane)?;
    plane.ingest(position(BookSide::Local, 600, t0()))?;
    plane.ingest(position(BookSide::Remote, 600, t0()))?;
    Ok(report(
        "risk breach",
        &plane,
        "position 600 exceeds max_position 500",
    ))
}

pub fn scenario_kill() -> Result<ScenarioReport, ControlError> {
    let mut plane = memory_plane(t0());
    apply_healthy(&mut plane)?;
    plane.kill("operator emergency stop".to_string())?;
    let resume = plane.resume();
    let resume_blocked = resume.is_err();
    let detail = format!(
        "resume_blocked={resume_blocked} state={}",
        plane.runtime().system_state
    );
    Ok(report("kill switch", &plane, &detail))
}

pub fn scenario_daily_loss_kill() -> Result<ScenarioReport, ControlError> {
    let mut plane = memory_plane(t0());
    apply_healthy(&mut plane)?;
    plane.ingest(DomainEvent::System(SystemEvent::SetDailyLoss {
        loss: Decimal::from(300),
    }))?;
    Ok(report(
        "daily loss kill",
        &plane,
        "loss 300 exceeds max_daily_loss 250",
    ))
}

fn report<S: StateStore>(
    name: &'static str,
    plane: &ControlPlane<S>,
    detail: &str,
) -> ScenarioReport {
    let gate = plane.trade_gate();
    ScenarioReport {
        name,
        system_state: gate.system_state,
        permission: gate.permission,
        decision: gate.decision,
        detail: detail.to_string(),
    }
}

pub fn print_reports(reports: &[ScenarioReport]) {
    for report in reports {
        println!(
            "{:<40} state={:<12} permission={:<28} decision={:<24} {}",
            report.name, report.system_state, report.permission, report.decision, report.detail
        );
    }
}
