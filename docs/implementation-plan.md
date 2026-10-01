# Implementation plan

Inspection happened before the control-plane code existed. The repository contained `README.md` and git metadata only: no `Cargo.toml`, `src/`, `tests/`, `examples/`, or `.github/`.

The README described a production-oriented control plane that answers whether a Polymarket trading system can safely continue. It named health, reconciliation, risk, recovery, alerts, and pause / kill behavior. It proposed a multi-crate layout and listed PostgreSQL, dashboards, and webhooks as later work. The only roadmap item marked done was the architecture write-up. Status was early development.

This crate implements that product in one library. The README said to stay small until a split is justified, so the suggested `crates/` layout was not created.

## Gaps that the code closes

| README concept | Before | After |
| --- | --- | --- |
| System health | Described | `SystemState`: Healthy, Degraded, Paused, Recovering, Failed, plus Kill as a latched control state |
| Trading permission | Implied by "can the system continue" | `TradingPermission` and `TradeDecision` evaluated from policy, not from socket connectivity |
| Orders, fills, positions, exposure | Named | Separate books on `RuntimeState` |
| Risk limits | Example YAML | `RiskConfig` evaluated by the risk engine |
| Reconciliation | Named | First-class compare of local and remote books |
| Recovery | Named | Staged recovery that does not treat reconnect as recovered |
| Alerts | Named | Typed alerts with deduplication |
| Persistence | SQLite or PostgreSQL mentioned | In-memory store and SQLite snapshot with migration `001` |
| Polymarket transport | Implied | Optional read-only adapter on `polymarket_client_sdk_v2` 0.7.0 |

## Modules

```text
src/domain/          typed state, permission, health reasons, risk, reconciliation, alerts, recovery, events
src/control/         commands, transition rules, policy, orchestrator
src/health/          freshness, connection effectiveness, assessment
src/risk/            exposure and limit evaluation
src/reconciliation/  local versus remote compare
src/recovery/        staged rebuild
src/alerts/          raise, resolve, dedup
src/repository/      memory, sqlite, failing store
src/adapters/        source traits, mock sources, optional Polymarket reads
src/telemetry.rs     structured transition logs
```

## Data flow

```text
Polymarket or a mock source
        |
        v
DomainEvent  (orders, fills, positions, market data, connection, execution, system)
        |
        v
ControlPlane::ingest
        |
        v
RuntimeState books
        |
        +-- reconcile local vs remote
        +-- recompute exposure
        +-- evaluate risk
        +-- assess health and trading permission
        +-- sync alerts
        |
        v
StateStore (memory or SQLite)
        |
        v
TradeGate: ALLOW | BLOCK | REQUIRE_RECONCILIATION | REQUIRE_RECOVERY
```

External I/O stays behind `MarketDataSource`, `OrderStateSource`, `TradeStateSource`, `PositionSource`, `ExecutionStateSource`, `ConnectionSource`, and `AuthoritativeStateSource`. Tests and the demo use mocks. The Polymarket module is compiled only with `--features polymarket`.

## State model

`RuntimeState` keeps separate maps for orders, fills, positions (local and remote), market data, and connections, plus exposure, risk, reconciliation, execution, recovery, alerts, and transitions.

A position mismatch can exist while risk is `UNKNOWN` and the system is `PAUSED`. Those are different fields.

Startup and reload begin at `PAUSED` with positions unknown and execution not checked. Restart does not mean healthy.

## Persistence

SQLite migration `migrations/001_init.sql` stores one JSON snapshot (`control_snapshot.id = 1`) and projects `state_transitions` and `alerts`. Load uses the snapshot. Every persisted object carries timestamps inside the snapshot.

PostgreSQL is not implemented.

## Health

`health::assess` returns a `SystemHealth` whose reasons are an enum, not one string. Inputs: connection effectiveness, market-data freshness, reconciliation, execution verdict, risk status, persistence, operator pause, recovery, kill latch, and unresolved critical condition alerts.

`CONNECTED` is not `HEALTHY`. A socket with no events, or with events older than `max_connection_event_age_ms`, is `STALE`.

## Risk

`RiskConfig` holds `max_position`, `max_market_exposure`, `max_total_exposure`, `max_daily_loss`, `max_execution_failures`, and `max_market_data_age_ms`, plus warning ratio and breach actions. Defaults match the README example (500, 1000, 5000, 250, 5 failures, 5000 ms). Daily loss defaults to kill. Other breaches default to pause.

Findings carry rule, observed value, threshold, timestamp, scope, and severity. Unknown positions or an unverified mark fail closed as `UNKNOWN` for the exposure rules.

## Reconciliation

Triggers include disconnect, restart, event gap, unexpected API response, mismatch, stale state, and execution uncertainty. Results are `MATCH`, `MISMATCH`, `UNKNOWN`, `RECOVERED`, and `FAILED`. A mismatch stores entity type, id, expected, observed, difference (`observed - expected`), timestamp, source, reason, and severity.

Empty order and fill books match. Unknown positions do not.

## Recovery

```text
EVENT GAP / DISCONNECT
        |
        v
PAUSE affected trading
        |
        v
RECONNECT          (this does not clear recovery)
        |
        v
FETCH authoritative state
        |
        v
RECONCILE orders, fills, positions
        |
        v
RECOMPUTE exposure
        |
        v
RECHECK risk
        |
        +-- checks pass and connection is ready -> RESUME
        +-- otherwise -> remain PAUSED
```

A recovery session records id, start, end, trigger, stages, failures, and result. Fetch failure marks positions and execution unknown.

## Alerts

Condition alerts (`STATE_MISMATCH`, `MARKET_DATA_STALE`, `WEBSOCKET_DISCONNECTED`, `RISK_LIMIT_BREACHED`, `TRADING_PAUSED`, `KILL_SWITCH_TRIGGERED`) are deduplicated by type, scope, and entity. The same open condition updates in place. Point events such as reconciliation started are not treated as a standing block. An externally raised critical alert blocks trading until it is resolved.

## Tests

`tests/suite.rs` covers health, permission, risk rules, reconciliation, websocket states, recovery, alerts, commands, and fail-closed injection. `tests/persistence.rs` reloads SQLite and checks that pause, alerts, transitions, stale data, and in-progress recovery survive restart. `src/control/transitions.rs` table-tests allowed and rejected transitions.

## Intentionally not in this crate

- Order placement, cancellation, or signing
- Private-key or API-credential storage
- A trading strategy
- A copy of the execution-verifier engine
- A long-running WebSocket task owned by this process
- PostgreSQL, dashboards, and outbound webhooks
