use chrono::{DateTime, Utc};

use crate::alerts::{self, AlertDraft};
use crate::clock::Clock;
use crate::domain::{
    Actor, Alert, AlertType, AuthoritativeSnapshot, BookSide, Command, ConnectionEventKind,
    ConnectionHealth, ConnectionSnapshot, DomainEvent, ExecutionSnapshot, ExecutionVerdict,
    HealthReason, KillRecord, MarketDataSnapshot, OrderLifecycle, RecoveryTrigger, Severity,
    StateTransition, SystemState, TradeDecision, TradeGate, TradingPermission, TransitionContext,
    TransitionKind, TransitionReason,
};
use crate::error::ControlError;
use crate::health::{self, Assessment};
use crate::reconciliation::{self, mark_recovered};
use crate::recovery::{self, RecoveryRequest};
use crate::repository::StateStore;
use crate::risk::{self, RiskInput};
use crate::runtime::RuntimeState;
use crate::telemetry;

use super::transitions::apply_transition;
use super::ControlPolicy;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Evaluation {
    pub system_state: SystemState,
    pub permission: TradingPermission,
    pub decision: TradeDecision,
    pub reasons: Vec<HealthReason>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandOutcome {
    pub idempotent: bool,
    pub evaluation: Evaluation,
}

pub struct ControlPlane<S: StateStore> {
    store: S,
    policy: ControlPolicy,
    clock: Clock,
    runtime: RuntimeState,
}

impl<S: StateStore> ControlPlane<S> {
    pub fn new(store: S, policy: ControlPolicy, now: DateTime<Utc>) -> Result<Self, ControlError> {
        let mut plane = Self {
            store,
            policy,
            clock: Clock::manual(now),
            runtime: RuntimeState::initial(now),
        };
        plane.reevaluate(None, Actor::System, TransitionReason::StartupUnverified)?;
        Ok(plane)
    }

    /// Load a snapshot if one exists. Restart does not imply healthy.
    pub fn open(store: S, policy: ControlPolicy, now: DateTime<Utc>) -> Result<Self, ControlError> {
        let loaded = store.load()?;
        let runtime = loaded.unwrap_or_else(|| RuntimeState::initial(now));
        let mut plane = Self {
            store,
            policy,
            clock: Clock::manual(now),
            runtime,
        };
        plane.runtime.updated_at = now;
        plane.reevaluate(None, Actor::System, TransitionReason::StartupUnverified)?;
        Ok(plane)
    }

    pub fn runtime(&self) -> &RuntimeState {
        &self.runtime
    }

    pub fn policy(&self) -> &ControlPolicy {
        &self.policy
    }

    pub fn store(&self) -> &S {
        &self.store
    }

    pub fn set_policy(&mut self, policy: ControlPolicy) -> Result<Evaluation, ControlError> {
        self.policy = policy;
        self.reevaluate(None, Actor::System, TransitionReason::DegradedWarning)
    }

    pub fn set_now(&mut self, now: DateTime<Utc>) -> Result<Evaluation, ControlError> {
        self.clock.set(now)?;
        self.reevaluate(None, Actor::HealthEngine, TransitionReason::MarketDataStale)
    }

    pub fn trade_gate(&self) -> TradeGate {
        TradeGate {
            decision: self.runtime.decision,
            permission: self.runtime.permission,
            system_state: self.runtime.system_state,
            reasons: self.latest_reasons(),
        }
    }

    pub fn evaluation(&self) -> Evaluation {
        Evaluation {
            system_state: self.runtime.system_state,
            permission: self.runtime.permission,
            decision: self.runtime.decision,
            reasons: self.latest_reasons(),
        }
    }

    pub fn ingest(&mut self, event: DomainEvent) -> Result<Evaluation, ControlError> {
        let now = self.clock.now()?;
        self.runtime.updated_at = now;
        if !self.apply_event(event, now) {
            return Ok(self.evaluation());
        }
        self.reevaluate(None, Actor::System, TransitionReason::ChecksPassed)
    }

    pub fn command(&mut self, command: Command) -> Result<CommandOutcome, ControlError> {
        match command {
            Command::Pause { reason } => self.pause(reason),
            Command::Resume => self.resume(),
            Command::Kill { reason } => self.kill(reason),
            Command::Reconcile => self.reconcile_command(),
            Command::Recover { trigger } => Err(ControlError::Source(format!(
                "recover ({trigger}) needs an authoritative snapshot; call recover_with"
            ))),
        }
    }

    pub fn pause(&mut self, reason: String) -> Result<CommandOutcome, ControlError> {
        if self.runtime.kill.is_some() {
            return Ok(CommandOutcome {
                idempotent: true,
                evaluation: self.evaluation(),
            });
        }
        if self.runtime.operator_paused && self.runtime.system_state == SystemState::Paused {
            return Ok(CommandOutcome {
                idempotent: true,
                evaluation: self.evaluation(),
            });
        }
        self.runtime.operator_paused = true;
        let evaluation = self.reevaluate(
            Some(TransitionKind::Pause),
            Actor::Operator,
            TransitionReason::OperatorPause,
        )?;
        if !reason.is_empty() {
            if let Some(last) = self.runtime.transitions.last_mut() {
                if last.reason == TransitionReason::OperatorPause {
                    last.detail = reason;
                }
            }
        }
        self.persist()?;
        Ok(CommandOutcome {
            idempotent: false,
            evaluation,
        })
    }

    pub fn resume(&mut self) -> Result<CommandOutcome, ControlError> {
        if self.runtime.kill.is_some() {
            return Err(ControlError::BlockedByKill);
        }
        if self.runtime.system_state == SystemState::Healthy
            && self.runtime.permission == TradingPermission::TradingAllowed
            && !self.runtime.operator_paused
            && !self.runtime.recovery_required
        {
            return Ok(CommandOutcome {
                idempotent: true,
                evaluation: self.evaluation(),
            });
        }
        let previous_hold = self.runtime.operator_paused;
        self.runtime.operator_paused = false;
        self.recompute();
        let assessment = health::assess(&self.runtime, &self.policy);
        if self.runtime.recovery_required || !assessment.gates_open || self.runtime.kill.is_some() {
            self.runtime.operator_paused = previous_hold;
            self.recompute();
            let blocked = health::assess(&self.runtime, &self.policy);
            self.runtime.permission = blocked.permission;
            self.runtime.decision = blocked.decision;
            let reasons = blocked
                .health
                .reasons
                .iter()
                .map(|r| r.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(ControlError::ResumeBlocked {
                reasons: if reasons.is_empty() {
                    "gates closed".to_string()
                } else {
                    reasons
                },
            });
        }
        let evaluation = self.finish_assessment(
            assessment,
            Some(TransitionKind::Resume),
            Actor::Operator,
            TransitionReason::OperatorResume,
        )?;
        Ok(CommandOutcome {
            idempotent: false,
            evaluation,
        })
    }

    pub fn kill(&mut self, reason: String) -> Result<CommandOutcome, ControlError> {
        if self.runtime.kill.is_some() {
            return Ok(CommandOutcome {
                idempotent: true,
                evaluation: self.evaluation(),
            });
        }
        let now = self.clock.now()?;
        self.runtime.updated_at = now;
        self.runtime.kill = Some(KillRecord {
            reason: reason.clone(),
            at: now,
            actor: Actor::Operator,
        });
        self.runtime.operator_paused = false;
        let evaluation = self.reevaluate(
            Some(TransitionKind::Kill),
            Actor::Operator,
            TransitionReason::OperatorKill,
        )?;
        if let Some(last) = self.runtime.transitions.last_mut() {
            if last.reason == TransitionReason::OperatorKill {
                last.detail = reason;
            }
        }
        self.persist()?;
        Ok(CommandOutcome {
            idempotent: false,
            evaluation,
        })
    }

    pub fn reconcile_command(&mut self) -> Result<CommandOutcome, ControlError> {
        let before = self.runtime.transitions.len();
        let evaluation = self.reevaluate(
            None,
            Actor::Reconciler,
            TransitionReason::ReconciliationRequired,
        )?;
        let now = self.runtime.updated_at;
        let status = self.runtime.reconciliation.status.to_string();
        alerts::raise_condition(
            &mut self.runtime.alerts,
            AlertDraft {
                event_type: AlertType::ReconciliationStarted,
                severity: Severity::Info,
                scope: "system".to_string(),
                entity_id: None,
                message: "reconciliation started".to_string(),
                metadata: serde_json::json!({ "status": status }),
            },
            now,
        );
        alerts::raise_condition(
            &mut self.runtime.alerts,
            AlertDraft {
                event_type: AlertType::ReconciliationCompleted,
                severity: Severity::Info,
                scope: "system".to_string(),
                entity_id: Some(status.clone()),
                message: format!("reconciliation completed: {status}"),
                metadata: serde_json::json!({ "status": status }),
            },
            now,
        );
        self.persist()?;
        Ok(CommandOutcome {
            idempotent: self.runtime.transitions.len() == before,
            evaluation,
        })
    }

    pub fn recover_with(
        &mut self,
        trigger: RecoveryTrigger,
        fetch: Result<AuthoritativeSnapshot, ControlError>,
    ) -> Result<CommandOutcome, ControlError> {
        if self.runtime.kill.is_some() {
            return Err(ControlError::BlockedByKill);
        }
        if self.runtime.system_state == SystemState::Healthy
            && self.runtime.permission == TradingPermission::TradingAllowed
            && !self.runtime.recovery_required
            && self
                .runtime
                .recovery
                .as_ref()
                .is_none_or(|session| !session.in_progress())
        {
            return Ok(CommandOutcome {
                idempotent: true,
                evaluation: self.evaluation(),
            });
        }

        let now = self.clock.now()?;
        self.runtime.updated_at = now;
        self.runtime.recovery_required = true;
        let session = crate::domain::RecoverySession::start(trigger, now);
        self.runtime.recovery = Some(session);
        self.recompute();
        let entering = health::assess(&self.runtime, &self.policy);
        self.converge(
            &entering,
            Some(TransitionKind::RecoveryStart),
            Actor::RecoveryEngine,
            TransitionReason::RecoveryStarted,
        )?;

        let effect = recovery::execute(RecoveryRequest {
            trigger,
            now,
            policy: &self.policy,
            orders_local: &self.runtime.orders_local,
            fills_local: &self.runtime.fills_local,
            positions_local: &self.runtime.positions_local,
            market_data: &self.runtime.market_data,
            daily_loss: self.runtime.daily_loss,
            execution_failures: self.runtime.execution.consecutive_failures,
            connection_ready: self.runtime.connections.values().any(|conn| {
                conn.effective == crate::domain::ConnectionHealth::Connected
                    && conn.reconnect_count < self.policy.risk.reconnect_loop_threshold
            }),
            fetch,
        });

        if effect.apply_remote {
            self.runtime.orders_remote = effect.orders;
            self.runtime.fills_remote = effect.fills;
            self.runtime.positions_remote = effect.positions;
            self.runtime.positions_unknown = effect.positions_unknown;
            self.runtime.execution.verdict = effect.execution;
            self.runtime.execution.execution_id = effect.execution_id;
            self.runtime.execution.updated_at = now;
        } else {
            self.runtime.positions_unknown = true;
            self.runtime.execution.verdict = ExecutionVerdict::Unknown;
        }
        self.runtime.recovery = Some(effect.session);
        if effect.latch_kill && self.runtime.kill.is_none() {
            self.runtime.kill = Some(KillRecord {
                reason: "MAX_DAILY_LOSS".to_string(),
                at: now,
                actor: Actor::RiskEngine,
            });
        }
        self.runtime.recovery_required = !(effect.checks_passed && self.runtime.kill.is_none());

        let reason = if effect.checks_passed {
            TransitionReason::RecoveryCompleted
        } else {
            TransitionReason::RecoveryFailed
        };
        let evaluation = self.reevaluate(
            Some(TransitionKind::RecoveryComplete),
            Actor::RecoveryEngine,
            reason,
        )?;
        let now = self.runtime.updated_at;
        alerts::push_event(
            &mut self.runtime.alerts,
            AlertDraft {
                event_type: AlertType::ReconciliationStarted,
                severity: Severity::Info,
                scope: "recovery".to_string(),
                entity_id: Some(trigger.to_string()),
                message: "recovery reconciliation started".to_string(),
                metadata: serde_json::json!({ "trigger": trigger.to_string() }),
            },
            now,
        );
        alerts::push_event(
            &mut self.runtime.alerts,
            AlertDraft {
                event_type: AlertType::ReconciliationCompleted,
                severity: if effect.checks_passed {
                    Severity::Info
                } else {
                    Severity::Critical
                },
                scope: "recovery".to_string(),
                entity_id: Some(self.runtime.reconciliation.status.to_string()),
                message: format!("recovery finished checks_passed={}", effect.checks_passed),
                metadata: serde_json::json!({
                    "checks_passed": effect.checks_passed,
                    "recovery_id": self.runtime.recovery.as_ref().map(|s| s.id.to_string()),
                }),
            },
            now,
        );
        self.persist()?;
        Ok(CommandOutcome {
            idempotent: false,
            evaluation,
        })
    }

    pub fn raise_external_alert(&mut self, mut alert: Alert) -> Result<Evaluation, ControlError> {
        if alert.dedup_key.is_empty() {
            alert.dedup_key =
                Alert::condition_key(alert.event_type, &alert.scope, alert.entity_id.as_deref());
        }
        let now = self.clock.now()?;
        if alerts::raise_condition(
            &mut self.runtime.alerts,
            AlertDraft {
                event_type: alert.event_type,
                severity: alert.severity,
                scope: alert.scope,
                entity_id: alert.entity_id,
                message: alert.message,
                metadata: alert.metadata,
            },
            now,
        ) {
            self.reevaluate(None, Actor::System, TransitionReason::CriticalAlert)
        } else {
            Ok(self.evaluation())
        }
    }

    fn apply_event(&mut self, event: DomainEvent, now: DateTime<Utc>) -> bool {
        match event {
            DomainEvent::Order(event) => {
                let book = match event.side_of_book {
                    BookSide::Local => &mut self.runtime.orders_local,
                    BookSide::Remote => &mut self.runtime.orders_remote,
                };
                if event.order.status == OrderLifecycle::Rejected {
                    alerts::raise_condition(
                        &mut self.runtime.alerts,
                        AlertDraft {
                            event_type: AlertType::OrderFailure,
                            severity: Severity::Warning,
                            scope: "order".to_string(),
                            entity_id: Some(event.order.id.clone()),
                            message: format!("order {} rejected", event.order.id),
                            metadata: serde_json::json!({ "order_id": event.order.id }),
                        },
                        now,
                    );
                }
                book.insert(event.order.id.clone(), event.order);
                true
            }
            DomainEvent::Fill(event) => {
                let book = match event.side_of_book {
                    BookSide::Local => &mut self.runtime.fills_local,
                    BookSide::Remote => &mut self.runtime.fills_remote,
                };
                if book.contains_key(&event.fill.id) {
                    return false;
                }
                let order_id = event.fill.order_id.clone();
                let size = event.fill.size;
                book.insert(event.fill.id.clone(), event.fill);
                if let Some(order_id) = order_id {
                    if let Some(order) = self
                        .runtime
                        .orders_local
                        .get(&order_id)
                        .cloned()
                        .or_else(|| self.runtime.orders_remote.get(&order_id).cloned())
                    {
                        if size < order.original_size && order.size_matched < order.original_size {
                            alerts::raise_condition(
                                &mut self.runtime.alerts,
                                AlertDraft {
                                    event_type: AlertType::PartialFill,
                                    severity: Severity::Warning,
                                    scope: "order".to_string(),
                                    entity_id: Some(order_id),
                                    message: "partial fill".to_string(),
                                    metadata: serde_json::json!({
                                        "fill_size": size.to_string(),
                                        "original_size": order.original_size.to_string(),
                                    }),
                                },
                                now,
                            );
                        }
                    }
                }
                true
            }
            DomainEvent::Position(event) => {
                if event.unknown {
                    self.runtime.positions_unknown = true;
                    return true;
                }
                self.runtime.positions_unknown = false;
                let book = match event.side_of_book {
                    BookSide::Local => &mut self.runtime.positions_local,
                    BookSide::Remote => &mut self.runtime.positions_remote,
                };
                book.insert(event.position.token_id.clone(), event.position);
                true
            }
            DomainEvent::MarketData(event) => {
                let snap = self
                    .runtime
                    .market_data
                    .entry(event.token_id.clone())
                    .or_insert(MarketDataSnapshot {
                        token_id: event.token_id.clone(),
                        market_id: event.market_id.clone(),
                        last_event_at: None,
                        last_snapshot_at: None,
                        best_bid: None,
                        best_ask: None,
                        last_trade_price: None,
                        freshness: crate::domain::Freshness::Unknown,
                    });
                snap.market_id = event.market_id;
                snap.best_bid = event.best_bid.or(snap.best_bid);
                snap.best_ask = event.best_ask.or(snap.best_ask);
                snap.last_trade_price = event.last_trade_price.or(snap.last_trade_price);
                snap.last_event_at = Some(event.at);
                if event.snapshot {
                    snap.last_snapshot_at = Some(event.at);
                }
                true
            }
            DomainEvent::Connection(event) => {
                let entry = self.runtime.connections.entry(event.id.clone()).or_insert(
                    ConnectionSnapshot {
                        id: event.id.clone(),
                        reported: ConnectionHealth::Disconnected,
                        effective: ConnectionHealth::Disconnected,
                        connected_at: None,
                        last_event_at: None,
                        reconnect_count: 0,
                        last_disconnect_at: None,
                        last_error: None,
                        updated_at: now,
                    },
                );
                match event.kind {
                    ConnectionEventKind::Connected => {
                        let was_down = matches!(
                            entry.reported,
                            ConnectionHealth::Disconnected
                                | ConnectionHealth::Reconnecting
                                | ConnectionHealth::Failed
                        );
                        if was_down && entry.connected_at.is_some() {
                            entry.reconnect_count = entry.reconnect_count.saturating_add(1);
                        }
                        entry.reported = ConnectionHealth::Connected;
                        entry.connected_at = Some(event.at);
                        entry.last_error = None;
                    }
                    ConnectionEventKind::Disconnected { error } => {
                        entry.reported = ConnectionHealth::Disconnected;
                        entry.last_disconnect_at = Some(event.at);
                        entry.last_error = error;
                        self.runtime.recovery_required = true;
                    }
                    ConnectionEventKind::Reconnecting { error } => {
                        entry.reported = ConnectionHealth::Reconnecting;
                        entry.last_error = error;
                        self.runtime.recovery_required = true;
                    }
                    ConnectionEventKind::EventReceived => {
                        entry.last_event_at = Some(event.at);
                    }
                    ConnectionEventKind::Failed { error } => {
                        entry.reported = ConnectionHealth::Failed;
                        entry.last_error = Some(error);
                        self.runtime.recovery_required = true;
                    }
                }
                entry.updated_at = now;
                true
            }
            DomainEvent::Transaction(event) => {
                self.runtime.execution.detail = format!(
                    "transaction {} for execution {}",
                    event.transaction_id, event.execution_id
                );
                self.runtime.execution.execution_id = Some(event.execution_id);
                self.runtime.execution.updated_at = event.at;
                true
            }
            DomainEvent::Execution(event) => {
                if event.failed {
                    self.runtime.execution.consecutive_failures = self
                        .runtime
                        .execution
                        .consecutive_failures
                        .saturating_add(1);
                } else if event.verdict == ExecutionVerdict::Verified {
                    self.runtime.execution.consecutive_failures = 0;
                }
                self.runtime.execution.execution_id = Some(event.execution_id);
                self.runtime.execution.verdict = event.verdict;
                self.runtime.execution.detail = event.detail;
                self.runtime.execution.updated_at = event.at;
                true
            }
            DomainEvent::System(event) => {
                use crate::domain::SystemEvent;
                match event {
                    SystemEvent::EventGap { .. } | SystemEvent::UnexpectedApi { .. } => {
                        self.runtime.recovery_required = true;
                    }
                    SystemEvent::SetDailyLoss { loss } => {
                        self.runtime.daily_loss = loss;
                    }
                }
                true
            }
        }
    }

    fn reevaluate(
        &mut self,
        kind: Option<TransitionKind>,
        actor: Actor,
        fallback_reason: TransitionReason,
    ) -> Result<Evaluation, ControlError> {
        let now = self.clock.now()?;
        self.runtime.updated_at = now;
        self.recompute();
        self.latch_risk_kill(now);
        let assessment = health::assess(&self.runtime, &self.policy);
        let reason = if self
            .runtime
            .kill
            .as_ref()
            .is_some_and(|kill| kill.actor == Actor::RiskEngine)
        {
            TransitionReason::DailyLossKill
        } else {
            primary_reason(&assessment.health.reasons).unwrap_or(fallback_reason)
        };
        let actor = match reason {
            TransitionReason::RiskLimitBreached
            | TransitionReason::DailyLossKill
            | TransitionReason::RiskUnknown => Actor::RiskEngine,
            TransitionReason::PositionMismatch => Actor::Reconciler,
            TransitionReason::OperatorPause
            | TransitionReason::OperatorResume
            | TransitionReason::OperatorKill => actor,
            TransitionReason::RecoveryStarted
            | TransitionReason::RecoveryCompleted
            | TransitionReason::RecoveryFailed => actor,
            _ => actor,
        };
        self.finish_assessment(assessment, kind, actor, reason)
    }

    fn finish_assessment(
        &mut self,
        assessment: Assessment,
        kind: Option<TransitionKind>,
        actor: Actor,
        reason: TransitionReason,
    ) -> Result<Evaluation, ControlError> {
        self.runtime.permission = assessment.permission;
        self.runtime.decision = assessment.decision;
        self.converge(&assessment, kind, actor, reason)?;
        alerts::sync_conditions(
            &mut self.runtime.alerts,
            &assessment.health.reasons,
            self.runtime.system_state,
            self.runtime.permission,
            self.runtime.updated_at,
        );
        let refreshed = health::assess(&self.runtime, &self.policy);
        self.runtime.permission = refreshed.permission;
        self.runtime.decision = refreshed.decision;
        if refreshed.health.state != self.runtime.system_state {
            self.converge(&refreshed, kind, actor, reason)?;
        }
        telemetry::log_permission(&self.runtime);
        self.persist()?;
        Ok(self.evaluation())
    }

    fn recompute(&mut self) {
        let now = self.runtime.updated_at;
        health::refresh_market_data(&mut self.runtime.market_data, now, &self.policy);
        health::refresh_connections(&mut self.runtime.connections, now, &self.policy);
        let mut report = reconciliation::reconcile(reconciliation::ReconcileBooks {
            orders_local: &self.runtime.orders_local,
            orders_remote: &self.runtime.orders_remote,
            fills_local: &self.runtime.fills_local,
            fills_remote: &self.runtime.fills_remote,
            positions_local: &self.runtime.positions_local,
            positions_remote: &self.runtime.positions_remote,
            positions_unknown: self.runtime.positions_unknown,
            execution: self.runtime.execution.verdict,
            now,
        });
        if self
            .runtime
            .recovery
            .as_ref()
            .is_some_and(|session| session.result == Some(crate::domain::RecoveryResult::Completed))
            && report.status == crate::domain::ReconciliationStatus::Match
        {
            mark_recovered(&mut report);
        }
        self.runtime.reconciliation = report;
        let positions = if !self.runtime.positions_remote.is_empty() {
            &self.runtime.positions_remote
        } else {
            &self.runtime.positions_local
        };
        let mut exposure = risk::recompute_exposure(
            positions,
            &self.runtime.market_data,
            self.runtime.daily_loss,
            self.runtime.positions_unknown,
            now,
        );
        if !matches!(
            self.runtime.reconciliation.positions,
            crate::domain::ReconciliationStatus::Match
                | crate::domain::ReconciliationStatus::Recovered
        ) {
            exposure.verified = false;
        }
        self.runtime.exposure = exposure;
        self.runtime.risk = risk::evaluate(
            &self.policy,
            &RiskInput {
                positions,
                exposure: &self.runtime.exposure,
                market_data: &self.runtime.market_data,
                daily_loss: self.runtime.daily_loss,
                execution_failures: self.runtime.execution.consecutive_failures,
                positions_unknown: self.runtime.positions_unknown,
                now,
            },
        );
        if self.runtime.execution.execution_id.is_none() {
            self.runtime.execution = ExecutionSnapshot {
                updated_at: now,
                ..self.runtime.execution.clone()
            };
        }
    }

    fn latch_risk_kill(&mut self, now: DateTime<Utc>) {
        if self.runtime.kill.is_none() && self.runtime.risk.kill_breaches().next().is_some() {
            self.runtime.kill = Some(KillRecord {
                reason: "MAX_DAILY_LOSS".to_string(),
                at: now,
                actor: Actor::RiskEngine,
            });
        }
    }

    fn converge(
        &mut self,
        assessment: &Assessment,
        kind: Option<TransitionKind>,
        actor: Actor,
        reason: TransitionReason,
    ) -> Result<(), ControlError> {
        let mut target = assessment.health.state;
        let from = self.runtime.system_state;
        if from == SystemState::Failed
            && matches!(target, SystemState::Healthy | SystemState::Degraded)
        {
            self.runtime.recovery_required = true;
            target = SystemState::Paused;
        }
        let kind = kind.unwrap_or(if target == SystemState::Killed {
            TransitionKind::Kill
        } else if from == SystemState::Recovering && target != SystemState::Recovering {
            TransitionKind::RecoveryComplete
        } else {
            TransitionKind::Automatic
        });
        let ctx = TransitionContext {
            gates_open: assessment.gates_open,
            kill_latched: self.runtime.kill.is_some(),
            operator_paused: self.runtime.operator_paused,
            recovery_required: self.runtime.recovery_required,
            recovery_active: self
                .runtime
                .recovery
                .as_ref()
                .is_some_and(|session| session.in_progress()),
        };
        if from == target {
            return Ok(());
        }
        let next = apply_transition(from, target, kind, reason, &ctx)?;
        let detail = assessment
            .health
            .reasons
            .first()
            .map(|r| r.to_string())
            .unwrap_or_default();
        let transition = StateTransition {
            previous: from,
            next,
            reason,
            kind,
            actor,
            at: self.runtime.updated_at,
            detail,
        };
        telemetry::log_transition(&transition);
        self.runtime.transitions.push(transition);
        self.runtime.system_state = next;
        Ok(())
    }

    fn persist(&mut self) -> Result<(), ControlError> {
        if let Err(err) = self.store.save(&self.runtime) {
            if self.runtime.persistence_healthy {
                self.runtime.persistence_healthy = false;
                self.runtime.permission = TradingPermission::TradingBlocked;
                self.runtime.decision = TradeDecision::Block;
                if self.runtime.system_state != SystemState::Killed {
                    let from = self.runtime.system_state;
                    let ctx = TransitionContext {
                        kill_latched: self.runtime.kill.is_some(),
                        ..TransitionContext::default()
                    };
                    if let Ok(next) = apply_transition(
                        from,
                        SystemState::Failed,
                        TransitionKind::Automatic,
                        TransitionReason::PersistenceFailure,
                        &ctx,
                    ) {
                        if next != from {
                            self.runtime.transitions.push(StateTransition {
                                previous: from,
                                next,
                                reason: TransitionReason::PersistenceFailure,
                                kind: TransitionKind::Automatic,
                                actor: Actor::System,
                                at: self.runtime.updated_at,
                                detail: err.to_string(),
                            });
                            self.runtime.system_state = next;
                        }
                    }
                }
            }
            return Err(err);
        }
        Ok(())
    }

    fn latest_reasons(&self) -> Vec<HealthReason> {
        health::assess(&self.runtime, &self.policy).health.reasons
    }
}

fn primary_reason(reasons: &[HealthReason]) -> Option<TransitionReason> {
    for reason in reasons {
        let mapped = match reason {
            HealthReason::MarketDataStale { .. } => Some(TransitionReason::MarketDataStale),
            HealthReason::MarketDataUnknown => Some(TransitionReason::MarketDataUnknown),
            HealthReason::PositionMismatch { .. } | HealthReason::ReconciliationMismatch => {
                Some(TransitionReason::PositionMismatch)
            }
            HealthReason::PositionUnknown | HealthReason::ReconciliationIncomplete => {
                Some(TransitionReason::ReconciliationRequired)
            }
            HealthReason::ExecutionUnknown | HealthReason::ExecutionNotChecked => {
                Some(TransitionReason::ExecutionUnknown)
            }
            HealthReason::ExecutionInconsistent => Some(TransitionReason::ExecutionInconsistent),
            HealthReason::RiskBreached { rule } if rule == "MAX_DAILY_LOSS" => {
                Some(TransitionReason::DailyLossKill)
            }
            HealthReason::RiskBreached { .. } => Some(TransitionReason::RiskLimitBreached),
            HealthReason::RiskUnknown => Some(TransitionReason::RiskUnknown),
            HealthReason::RiskWarning => Some(TransitionReason::DegradedWarning),
            HealthReason::ConnectionDisconnected
            | HealthReason::ConnectionFailed
            | HealthReason::ConnectionMissing
            | HealthReason::ConnectionReconnecting
            | HealthReason::ConnectionStale
            | HealthReason::ConnectedWithoutEvents => Some(TransitionReason::ConnectionUnhealthy),
            HealthReason::ReconnectLoop { .. } => Some(TransitionReason::ReconnectLoop),
            HealthReason::PersistenceUnhealthy => Some(TransitionReason::PersistenceFailure),
            HealthReason::RecoveryRequired => Some(TransitionReason::EventGap),
            HealthReason::RecoveryInProgress => Some(TransitionReason::RecoveryStarted),
            HealthReason::KillSwitchActive => Some(TransitionReason::OperatorKill),
            HealthReason::OperatorPaused => Some(TransitionReason::OperatorPause),
            HealthReason::ReconciliationFailed => Some(TransitionReason::RecoveryFailed),
            HealthReason::CriticalAlert => Some(TransitionReason::CriticalAlert),
        };
        if mapped.is_some() {
            return mapped;
        }
    }
    None
}
