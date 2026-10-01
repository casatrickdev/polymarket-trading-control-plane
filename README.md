# Polymarket Trading Control Plane

**Health, risk, reconciliation, and recovery for automated Polymarket trading systems.**

by [Casatrick on Telegram](https://t.me/casatrick).

A trading bot decides what it wants to trade. This control plane decides whether the system is healthy enough to continue.

> The strategy decides what to trade.
> The control plane decides whether the system is safe enough to keep trading.

It does not choose markets, prices, or a strategy. It does not place orders, generate wallets, or read private keys.

**Status:** the library, in-memory store, SQLite store, mock sources, deterministic tests, and local demo are implemented. The Polymarket module is a read-only adapter compiled with `--features polymarket`. Items marked **PLANNED** are not implemented.

---

## 1. What the control plane is

`polymarket-trading-control-plane` is a Rust library that sits above a trading bot and an execution verifier. It keeps orders, fills, positions, exposure, market data, and connections as separate state, then answers one question:

**Is the trading system in a state where it can safely continue?**

## 2. What problem it solves

Once a bot holds positions, connectivity alone is not a safety check. The control plane tracks whether market data is fresh, whether local books match an authoritative snapshot, whether execution is verified, whether risk limits hold, and whether trading must pause until recovery finishes.

## 3. Architecture

```text
Polymarket
        |
        v
Market data / orders / trades / positions
        |
        v
State ingestion
        |
        v
Reconciliation
        |
        v
Health + risk
        |
        v
Control plane
        |
        v
Pause / Resume / Kill / Recover
        |
        v
Trading bot / execution layer
```

Transport stays behind traits (`MarketDataSource`, `OrderStateSource`, `TradeStateSource`, `PositionSource`, `ExecutionStateSource`, `ConnectionSource`, `AuthoritativeStateSource`). The domain tests run against mock sources with no network.

```text
Trading Bot
        |
        v
Execution
        |
        v
Execution Verifier
        |
        v
Reconciliation
        |
        v
Trading Control Plane
        |
        v
Risk / Health / Monitoring / Recovery
```

Related repositories:

- [Polymarket Trading Bot](https://github.com/casatrickdev/polymarket-trading-bot)
- [Polymarket Execution Verifier](https://github.com/casatrickdev/polymarket-execution-verifier)
- [Polymarket Trading Control Plane](https://github.com/casatrickdev/polymarket-trading-control-plane)

## 4. System health model

`SystemState` is an enum:

| State | Meaning |
| --- | --- |
| `HEALTHY` | Required checks pass and trading may be allowed |
| `DEGRADED` | Operating with warnings. Trading stays blocked unless policy explicitly allows it |
| `PAUSED` | Trading is blocked. Monitoring and reconciliation continue |
| `RECOVERING` | State is being rebuilt. New trading does not start |
| `FAILED` | A critical condition prevents normal operation, including persistence failure |
| `KILLED` | Emergency latch. Stronger than pause. Resume does not clear it |

A warning does not become `FAILED`. `KILLED` is a control state, not process termination.

## 5. Trading permission model

`TradingPermission` is `TRADING_ALLOWED`, `TRADING_BLOCKED`, or `TRADING_REQUIRES_VERIFICATION`.

The bot asks `ControlPlane::trade_gate` and receives:

| Decision | Meaning |
| --- | --- |
| `ALLOW` | Policy requirements are satisfied |
| `BLOCK` | Trading must not proceed |
| `REQUIRE_RECONCILIATION` | Books or execution are not trusted |
| `REQUIRE_RECOVERY` | A disconnect, gap, or unfinished recovery is outstanding |

Trading is allowed only when the connection is healthy, market data is fresh, execution is verified, positions are reconciled, exposure is verified, risk is healthy, and no critical failure is open. Unknown positions, unknown execution, unknown risk, stale market data, and incomplete reconciliation fail closed.

Socket connectivity does not grant permission. A successful reconnect does not return the system to `HEALTHY`.

## 6. Risk controls

`RiskConfig` is the policy. Evaluation does not embed the numbers.

| Limit | Default |
| --- | --- |
| `max_position` | 500 |
| `max_market_exposure` | 1000 |
| `max_total_exposure` | 5000 |
| `max_daily_loss` | 250 |
| `max_execution_failures` | 5 |
| `max_market_data_age_ms` | 5000 |

`RiskStatus` is `HEALTHY`, `WARNING`, `BREACHED`, or `UNKNOWN`. A finding records the rule, observed value, threshold, timestamp, scope (`TOKEN`, `MARKET`, `PORTFOLIO`, or `SYSTEM`), and severity.

Position and market exposure are per token or market. Total exposure and daily loss are portfolio scope. Execution-failure count is system scope. Daily loss defaults to the kill latch. Other breaches default to pause. A warning is raised above 80% of a limit.

Financial values use `rust_decimal::Decimal`.

## 7. Reconciliation

Reconciliation compares local and remote orders, fills, and positions. Status is `MATCH`, `MISMATCH`, `UNKNOWN`, `RECOVERED`, or `FAILED`.

A mismatch records entity type, identifier, expected value, observed value, difference (`observed - expected`), timestamp, source, reason, and severity. Example: expected position 100, observed 40, difference -60. That pauses trading and asks for reconciliation.

## 8. Recovery

Recovery is a session with an id, trigger, stages, failures, and a final result.

```text
EVENT GAP / DISCONNECT
        |
        v
PAUSE
        |
        v
RECONNECT
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
        +-- healthy -> RESUME
        +-- not healthy -> remain PAUSED
```

`RECONNECT` is not `RECOVERED`. Recovery also refuses to complete while the connection is still down, reconnecting, stale, or in a reconnect loop.

## 9. Alerts

| Type | Typical severity |
| --- | --- |
| `STATE_MISMATCH` | critical while open |
| `ORDER_FAILURE` | warning, or critical when raised externally |
| `PARTIAL_FILL` | info |
| `MARKET_DATA_STALE` | warning, critical if the system is already paused or failed |
| `WEBSOCKET_DISCONNECTED` | warning |
| `RECONCILIATION_STARTED` | info |
| `RECONCILIATION_COMPLETED` | info, or warning when recovery did not pass |
| `RISK_LIMIT_BREACHED` | critical while open |
| `TRADING_PAUSED` | warning |
| `KILL_SWITCH_TRIGGERED` | critical |

Severity is `INFO`, `WARNING`, or `CRITICAL`. Each alert has a timestamp, scope, message, structured metadata, and resolved or unresolved state. The same open condition updates one alert instead of appending duplicates.

## 10. Persistence

`MemoryStore` and `SqliteStore` implement `StateStore`. SQLite uses `migrations/001_init.sql`: one JSON snapshot plus projected transition and alert rows. Reload restores pause, unresolved alerts, and an in-progress recovery, then recalculates permission. Stale market data is detected against the new clock. Restart is not treated as healthy.

PostgreSQL is **PLANNED**.

## 11. Polymarket adapter

Optional feature `polymarket` depends on `polymarket_client_sdk_v2` 0.7.0 with features `clob`, `ws`, and `data`. It is not a default feature.

Implemented reads, with no signer:

- CLOB `ok`, `midpoint`, `order_book` on `https://clob-v2.polymarket.com`
- Data API `positions` and `trades` on `https://data-api.polymarket.com`
- Normalization of WebSocket `book`, `best_bid_ask`, `last_trade_price`, `order`, and `trade` messages into domain events
- Normalization of authenticated `OpenOrderResponse` and `TradeResponse` values the caller already fetched

Not implemented, on purpose:

- `post_order`, cancel, or any signing path
- Loading a private key or API secret
- A process-owned WebSocket subscription loop (**PLANNED**). Callers pass `WsMessage` into `events_from_ws`
- Authenticated CLOB `orders` / `trades` calls. Those methods require an authenticated SDK client this crate does not construct

Enable it with `cargo check --features polymarket`. Local tests and the demo do not need the feature or any credentials.

## 12. Execution verifier integration

This crate does not reimplement [polymarket-execution-verifier](https://github.com/casatrickdev/polymarket-execution-verifier).

Ingest `DomainEvent::Execution` with `ExecutionVerdict::Verified`, `Unknown`, or `Inconsistent`. `Unknown` and `Inconsistent` block trading and require reconciliation. `NotChecked` also blocks while `require_execution_verification` is true (the default).

## 13. Trading bot integration

This crate does not depend on [polymarket-trading-bot](https://github.com/casatrickdev/polymarket-trading-bot).

The bot calls `trade_gate()` and obeys `ALLOW`, `BLOCK`, `REQUIRE_RECONCILIATION`, or `REQUIRE_RECOVERY`. The gate includes the health reasons. The bot does not re-derive those rules.

Operator commands are `PAUSE`, `RESUME`, `KILL`, `RECONCILE`, and `RECOVER`.

- `PAUSE` is idempotent.
- `RESUME` fails while a kill latch, risk breach, unverified position, incomplete reconciliation, stale data, unhealthy connection, or unresolved critical execution state remains.
- `KILL` blocks trading until an operator path that this crate does not provide clears it. `RESUME` returns `BlockedByKill`.
- `RECONCILE` and a healthy `RECOVER` are idempotent.

## 14. Local demo

No credentials:

```bash
cargo run --example control_plane_demo
```

| Scenario | Result |
| --- | --- |
| Fresh data, healthy connection, matching positions, risk inside limits | `HEALTHY`, `TRADING_ALLOWED` |
| Market data older than the configured max age | `PAUSED` (default) or `DEGRADED` if policy says degrade. Trading stays blocked |
| Expected position 100, observed 40 | `PAUSED`, `REQUIRE_RECONCILIATION`, difference -60 |
| Disconnect, reconnect, recover, reconcile, verify, risk check | stays `PAUSED` after reconnect, then `HEALTHY` only after recovery passes |
| Position above `max_position` | `PAUSED`, `RISK_LIMIT_BREACHED` |
| Operator `KILL` | `KILLED`. A later `RESUME` stays blocked |

## 15. Tests

```bash
cargo test
cargo test --features polymarket
```

The suite covers health states, trading permission, each risk rule, reconciliation outcomes, websocket states including stale and reconnect loops, recovery success and failure, alert emit / dedup / resolve, command guardrails, fail-closed fault injection, and SQLite restart.

## 16. Current implementation status

Implemented:

- Typed system state, trading permission, and validated transitions
- Health, risk, reconciliation, recovery, and alert engines
- In-memory and SQLite persistence
- Mock sources and failure injection
- Credential-free demo
- Read-only Polymarket adapter behind the `polymarket` feature

**PLANNED:**

- PostgreSQL
- A library-owned WebSocket client task
- Dashboards, webhooks, and chat notifications
- Calling authenticated CLOB order or trade endpoints (the caller must supply an already authenticated SDK client; this crate will not create one)

Design notes are in [docs/implementation-plan.md](docs/implementation-plan.md).

## Develop

Rust 1.88 or newer.

```bash
cargo fmt --check
cargo test
cargo clippy --all-targets --all-features -- -D warnings
```

CI runs those checks. Copy `.env.example` if a surrounding process needs host placeholders. This library does not read that file and does not accept a private key.
