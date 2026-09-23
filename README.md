# Polymarket Trading Control Plane

> Production-oriented monitoring, risk controls, state health, and operational tooling for automated Polymarket trading systems.

**Status:** Early development "under private"

A trading bot does more than place orders.

Once a system is running with real positions, the important questions become:

* Are my orders actually in the state I think they are?
* Are my positions correct?
* Is market data still fresh?
* Did an execution fail or partially fill?
* Is the system still connected?
* Am I within my risk limits?
* Should the bot keep trading right now?

**Polymarket Trading Control Plane** is a small infrastructure layer designed to answer those questions.

The goal is not to build another trading strategy.

The goal is to build the **operational layer around a trading strategy**.

---

## Why this exists

A simple trading bot often looks like:

```text
Market Data
    ↓
Strategy
    ↓
  Order
```

A production trading system is closer to:

```text
Market Data
    ↓
Event Processing
    ↓
  State
    ↓
Strategy
    ↓
  Risk
    ↓
Execution
    ↓
Monitoring
    ↓
Recovery
```

The difficult failures are often silent.

For example:

```text
WebSocket disconnect
        ↓
Missed execution event
        ↓
Local state becomes stale
        ↓
Position is incorrect
        ↓
Strategy keeps trading
```

A system can remain online while its view of the market is wrong.

This project is designed to make those conditions visible and controllable.

---

## Core goals

The control plane focuses on five areas:

### 1. System health

Know whether the trading system is operating normally.

Track:

* WebSocket connection
* market-data freshness
* order stream freshness
* event-processing latency
* API availability
* last successful reconciliation
* process heartbeat

### 2. Trading state

Maintain an observable view of:

* open orders
* executed trades
* partial fills
* positions
* exposure
* realized PnL
* unrealized PnL
* recent execution activity

### 3. Risk controls

Provide explicit limits around:

* maximum position
* maximum market exposure
* maximum total exposure
* daily loss
* order size
* execution failure count
* stale-data conditions

### 4. Recovery

Detect conditions such as:

* WebSocket disconnect
* missed events
* stale state
* failed orders
* partial fills
* inconsistent position state
* API degradation

Then transition the system into a controlled state.

### 5. Operational control

Provide actions such as:

```text
PAUSE TRADING
RESUME TRADING
KILL SWITCH
RECONCILE STATE
```

The control plane should make it possible to stop trading **before an infrastructure problem becomes a trading problem**.

---

# Architecture

```text
                         POLYMARKET
                             │
              ┌──────────────┴──────────────┐
              │                             │
        WebSocket Streams              API / Reads
              │                             │
              ▼                             ▼
      ┌────────────────┐          ┌──────────────────┐
      │ Event Ingestion│          │ State Reconciler │
      └───────┬────────┘          └────────┬─────────┘
              │                            │
              └────────────┬───────────────┘
                           ▼
                  ┌──────────────────┐
                  │ State Store      │
                  │                  │
                  │ Orders           │
                  │ Trades           │
                  │ Positions        │
                  │ Exposure         │
                  │ PnL              │
                  │ Health           │
                  └────────┬─────────┘
                           │
            ┌──────────────┼──────────────┐
            ▼              ▼              ▼
       Risk Engine    Health Engine   Alert Engine
            │              │              │
            └──────────────┼──────────────┘
                           ▼
                  ┌──────────────────┐
                  │ Control Plane    │
                  │                  │
                  │ Pause            │
                  │ Resume           │
                  │ Kill Switch      │
                  │ Reconcile        │
                  └──────────────────┘
```

---

# Polymarket integration

The project is designed around Polymarket's current trading/data architecture.

Polymarket provides public market WebSocket streams and an authenticated user channel for order and trade events. Its current Rust client also exposes orderbook, price, order, and trade subscriptions, alongside Data and Gamma API clients.

The control plane uses those interfaces as inputs, while maintaining its own normalized operational state.

Conceptually:

```text
Polymarket
    ↓
Raw Events
    ↓
Normalized Events
    ↓
State Store
    ↓
Health / Risk / Control
```

The project does **not** attempt to replace the Polymarket CLOB.

It sits around the trading system that consumes it.

---

# State model

The system keeps separate concepts for:

```text
Orders
    ↓
Trades / Fills
    ↓
Positions
    ↓
Exposure
    ↓
Risk
```

This distinction matters.

An order is what the strategy requested.

A fill is what actually executed.

A position is what the account currently holds.

Exposure is the risk resulting from those positions.

These should not be derived from a single `order` object.

---

# Health model

Every subsystem should expose an explicit health state.

Example:

```text
SYSTEM: DEGRADED

Market data:        OK
Order stream:       OK
API connectivity:   OK
Position state:     MISMATCH
Last reconciliation: 42s ago
Risk engine:        WARNING
Trading:            PAUSED
```

Possible states:

```text
HEALTHY
DEGRADED
PAUSED
RECOVERING
FAILED
```

---

# Reconciliation

Real-time events are useful for low-latency state updates.

They should not be treated as the only mechanism for recovering state.

After conditions such as:

* WebSocket reconnect
* application restart
* suspected event gap
* unexpected API response
* state mismatch

the control plane should compare local state against current remote state.

```text
Local State
     │
     ├──────────────┐
     │              │
     ▼              ▼
  Orders         Positions
     │              │
     └──────┬───────┘
            │
            ▼
     Remote State
            │
            ▼
      Compare / Repair
            │
            ▼
      Verified State
```

A simplified discrepancy report might look like:

```text
POSITION MISMATCH

Market: BTC Up/Down

Local:
  +100

Remote:
  +40

Difference:
  -60

Status:
  REQUIRES_RECONCILIATION
```

The strategy should not automatically continue trading simply because the network connection has returned.

---

# Risk controls

The first version intentionally keeps risk controls simple.

Example:

```yaml
max_position_size: 500
max_market_exposure: 1000
max_total_exposure: 5000
max_daily_loss: 250
max_consecutive_failures: 5
max_market_data_age_ms: 5000
```

Example decision flow:

```text
Risk Check
    │
    ├── Position limit exceeded? ──→ PAUSE
    ├── Daily loss exceeded? ──────→ KILL
    ├── Data too old? ─────────────→ PAUSE
    ├── State mismatch? ───────────→ PAUSE
    └── Everything OK ──────────────→ ALLOW
```

The risk layer should be independent of the strategy.

That allows the same controls to protect different trading strategies.

---

# Kill switch

A kill switch is intentionally simple.

```text
                    KILL SWITCH
                         │
          ┌──────────────┴──────────────┐
          │                             │
     Stop new orders              Optional exit logic
          │
          ▼
      Freeze strategy
          │
          ▼
      Alert operator
```

Possible triggers:

* manual operator action
* daily loss threshold
* unexpected exposure
* persistent state mismatch
* repeated execution failures
* severe market-data staleness
* system health failure

The exact response should be configurable.

---

# Alerts

The control plane should produce machine-readable events for:

```text
STATE_MISMATCH
ORDER_FAILURE
PARTIAL_FILL
MARKET_DATA_STALE
WEBSOCKET_DISCONNECTED
RECONCILIATION_STARTED
RECONCILIATION_COMPLETED
RISK_LIMIT_BREACHED
TRADING_PAUSED
KILL_SWITCH_TRIGGERED
```

Example:

```json
{
  "event": "STATE_MISMATCH",
  "market": "example-market",
  "severity": "critical",
  "local_position": 100,
  "remote_position": 40,
  "action": "pause_trading"
}
```

This makes the same event usable by:

* CLI
* dashboard
* logs
* Telegram/Discord alerts
* webhooks
* future agent integrations

---

# Technology

Initial implementation:

```text
Rust
Tokio
WebSocket
REST APIs
SQLite / PostgreSQL
Structured logging
```

Potential components:

```text
crates/
├── ingest
├── events
├── state
├── reconcile
├── risk
├── health
├── alerts
└── control
```

The first release should stay small.

Avoid introducing distributed infrastructure before it is actually required.

---

# Project structure

```text
polymarket-trading-control-plane/
│
├── crates/
│   ├── ingest/
│   ├── events/
│   ├── state/
│   ├── reconcile/
│   ├── risk/
│   ├── health/
│   ├── alerts/
│   └── control/
│
├── examples/
│   ├── monitor/
│   ├── reconcile/
│   └── simulated_failure/
│
├── tests/
│   ├── state/
│   ├── reconciliation/
│   ├── risk/
│   └── recovery/
│
├── docs/
│   ├── architecture.md
│   ├── state-model.md
│   ├── recovery.md
│   └── risk.md
│
├── Cargo.toml
├── Cargo.lock
└── README.md
```

---

# Development roadmap

## Phase 1 - State monitoring

* [x] Project architecture
* [ ] WebSocket connection
* [ ] Event ingestion
* [ ] Order state
* [ ] Trade/fill state
* [ ] Position state
* [ ] Health state

## Phase 2 - Reconciliation

* [ ] Persist state
* [ ] Remote state reader
* [ ] State comparison
* [ ] Discrepancy detection
* [ ] Recovery after reconnect
* [ ] Recovery after restart

## Phase 3 - Risk

* [ ] Position limits
* [ ] Exposure limits
* [ ] Daily loss limit
* [ ] Data freshness limits
* [ ] Execution failure limits
* [ ] Risk state machine

## Phase 4 - Operational controls

* [ ] Pause
* [ ] Resume
* [ ] Kill switch
* [ ] Manual reconciliation
* [ ] Alert delivery

## Phase 5 - Observability

* [ ] Metrics
* [ ] Structured logs
* [ ] Latency tracking
* [ ] Health dashboard
* [ ] Execution timeline
* [ ] Historical incident replay

## Phase 6 - Integration

* [ ] Strategy adapter
* [ ] Execution adapter
* [ ] Webhook API
* [ ] External dashboard
* [ ] Agent/MCP integration

---

# Example workflow

A healthy system:

```text
Market data
    ↓
Event received
    ↓
State updated
    ↓
Risk check
    ↓
Strategy allowed
    ↓
Execution
```

A failed connection:

```text
WebSocket disconnect
    ↓
Health = DEGRADED
    ↓
Trading = PAUSED
    ↓
Reconnect
    ↓
Reconcile
    ↓
Verify orders
    ↓
Verify positions
    ↓
Verify risk
    ↓
Trading = RESUMED
```

A critical risk event:

```text
Daily loss limit exceeded
    ↓
Risk = CRITICAL
    ↓
Trading = PAUSED
    ↓
Cancel / protect according to policy
    ↓
Alert operator
```

---

# Failure scenarios

The project should eventually test at least:

| Scenario                | Expected behavior            |
| ----------------------- | ---------------------------- |
| WebSocket disconnect    | Mark degraded, reconnect     |
| Missed event            | Detect and reconcile         |
| Duplicate event         | Ignore safely                |
| Partial fill            | Update execution state       |
| Unknown order state     | Reconcile                    |
| Stale market data       | Pause affected trading       |
| API failure             | Enter degraded state         |
| Process restart         | Reload and reconcile         |
| Risk limit breach       | Pause or kill                |
| Reconciliation mismatch | Block trading until resolved |

---

# What this project is not

This project is **not**:

* a guaranteed-profit trading bot
* a trading strategy
* financial advice
* a copy-trading service
* a prediction model
* an investment product

It is infrastructure for building and operating automated trading systems.

---

# Why this matters

A strategy answers:

> **What should I trade?**

An execution engine answers:

> **How should I place the order?**

A control plane answers:

> **Can I trust the system enough to keep trading?**

That last question becomes increasingly important as trading systems become more automated.

---

# Design principles

### Correctness before convenience

Trading state should be explicit and auditable.

### Fail closed

When critical state cannot be trusted, the default should be to reduce or stop new risk.

### Events are inputs, not assumptions

The system should be able to recover when events are delayed, duplicated, or missed.

### Strategy-independent infrastructure

Risk, health and reconciliation should work independently of a particular strategy.

### Replayability

Important state transitions should eventually be replayable for debugging and testing.

### Small first

The first version should solve one operational problem well rather than become a complete trading platform.

---

# Intended users

This project is useful for developers building:

* Polymarket trading bots
* automated trading systems
* arbitrage systems
* market-making systems
* copy-trading systems
* quantitative trading infrastructure
* research-to-production trading pipelines

It can also serve as a foundation for custom trading infrastructure where execution reliability and operational controls matter.

---

# Long-term direction

The control plane can eventually become the operational layer between a strategy and an execution engine:

```text
            STRATEGY
                │
                ▼
        ┌───────────────┐
        │ CONTROL PLANE │
        │               │
        │ State         │
        │ Risk          │
        │ Health        │
        │ Reconciliation│
        │ Alerts        │
        │ Recovery      │
        └───────┬───────┘
                │
                ▼
          EXECUTION ENGINE
                │
                ▼
            POLYMARKET
```

Future extensions could include:

* historical replay
* strategy simulation
* execution analytics
* incident replay
* distributed workers
* portfolio-level controls
* external dashboards
* APIs
* agent integrations

---

# Status

**Early development.**

The initial objective is to establish a reliable state, health, reconciliation, and risk foundation before adding sophisticated strategy or execution logic.

---

# Contributing

Issues, architecture discussions, and improvements are welcome.

Areas of particular interest:

* state management
* reconciliation
* WebSocket reliability
* execution monitoring
* risk systems
* observability
* Rust trading infrastructure

---

# Keywords

`Polymarket` · `Polymarket API` · `Polymarket CLOB` · `Polymarket WebSocket` · `Polymarket trading bot` · `Polymarket trading system` · `Polymarket execution` · `Polymarket risk management` · `Polymarket monitoring` · `Polymarket reconciliation` · `algorithmic trading` · `trading infrastructure` · `Rust trading bot` · `automated trading` · `real-time trading systems`

---

## About Casatrick

Casatrick builds trading, data, and automation systems for Polymarket, with a focus on real-time infrastructure, execution, reliability, risk, and production engineering.

The goal of this project is to explore the infrastructure required to operate automated Polymarket trading systems reliably.

If you need help or code contact me at [Telegram](https://t.me/casatrick)
