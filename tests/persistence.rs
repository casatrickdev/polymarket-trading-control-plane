use chrono::Duration;
use polymarket_trading_control_plane::demo::{apply_healthy, position, t0, TOKEN};
use polymarket_trading_control_plane::domain::{
    AlertType, BookSide, RecoverySession, RecoveryTrigger, SystemState, TradingPermission,
};
use polymarket_trading_control_plane::repository::{SqliteStore, StateStore};
use polymarket_trading_control_plane::runtime::RuntimeState;
use polymarket_trading_control_plane::{ControlPlane, ControlPolicy};

#[test]
fn paused_state_alerts_and_transitions_survive_restart() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let path = file.path().to_path_buf();
    {
        let store = SqliteStore::open(&path).unwrap();
        let mut plane = ControlPlane::new(store, ControlPolicy::default(), t0()).unwrap();
        apply_healthy(&mut plane).unwrap();
        plane.ingest(position(BookSide::Remote, 40, t0())).unwrap();
        plane.pause("hold across restart".into()).unwrap();
        assert!(plane
            .runtime()
            .alerts
            .iter()
            .any(|alert| alert.event_type == AlertType::StateMismatch && !alert.resolved));
        assert!(!plane.runtime().transitions.is_empty());
    }
    let store = SqliteStore::open(&path).unwrap();
    let transitions = store.transitions().unwrap();
    let alerts = store.alerts().unwrap();
    assert!(!transitions.is_empty());
    assert!(alerts
        .iter()
        .any(|alert| { alert.event_type == AlertType::StateMismatch && !alert.resolved }));
    let plane = ControlPlane::open(store, ControlPolicy::default(), t0()).unwrap();
    assert_eq!(plane.runtime().system_state, SystemState::Paused);
    assert!(plane.runtime().operator_paused);
    assert_ne!(
        plane.trade_gate().permission,
        TradingPermission::TradingAllowed
    );
    assert!(plane
        .runtime()
        .alerts
        .iter()
        .any(|alert| alert.event_type == AlertType::StateMismatch && !alert.resolved));
    assert!(plane
        .runtime()
        .reconciliation
        .mismatches
        .iter()
        .any(|item| item.entity_id == TOKEN));
}

#[test]
fn restart_does_not_treat_stale_data_as_healthy() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let path = file.path().to_path_buf();
    {
        let store = SqliteStore::open(&path).unwrap();
        let mut plane = ControlPlane::new(store, ControlPolicy::default(), t0()).unwrap();
        apply_healthy(&mut plane).unwrap();
        assert_eq!(plane.runtime().system_state, SystemState::Healthy);
    }
    let mut policy = ControlPolicy::default();
    policy.risk.max_market_data_age_ms = 2_000;
    policy.risk.max_connection_event_age_ms = 60_000;
    let plane = ControlPlane::open(
        SqliteStore::open(&path).unwrap(),
        policy,
        t0() + Duration::seconds(4),
    )
    .unwrap();
    assert_ne!(plane.runtime().system_state, SystemState::Healthy);
    assert_ne!(
        plane.trade_gate().permission,
        TradingPermission::TradingAllowed
    );
    assert!(plane.trade_gate().reasons.iter().any(|reason| {
        matches!(
            reason,
            polymarket_trading_control_plane::domain::HealthReason::MarketDataStale { .. }
        )
    }));
}

#[test]
fn in_progress_recovery_stays_blocked_after_reload() {
    let file = tempfile::NamedTempFile::new().unwrap();
    let path = file.path().to_path_buf();
    let store = SqliteStore::open(&path).unwrap();
    let mut state = RuntimeState::initial(t0());
    state.system_state = SystemState::Recovering;
    state.recovery_required = true;
    state.recovery = Some(RecoverySession::start(
        RecoveryTrigger::ApplicationRestart,
        t0(),
    ));
    store.save(&state).unwrap();
    drop(store);
    let plane = ControlPlane::open(
        SqliteStore::open(&path).unwrap(),
        ControlPolicy::default(),
        t0(),
    )
    .unwrap();
    assert_eq!(plane.runtime().system_state, SystemState::Recovering);
    assert_ne!(
        plane.trade_gate().permission,
        TradingPermission::TradingAllowed
    );
    assert_ne!(
        plane.trade_gate().decision,
        polymarket_trading_control_plane::domain::TradeDecision::Allow
    );
}
