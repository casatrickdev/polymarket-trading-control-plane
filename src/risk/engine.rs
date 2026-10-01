use std::collections::BTreeMap;

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::control::{ControlPolicy, StaleDataAction};
use crate::domain::{
    BreachAction, ExposureSnapshot, Freshness, MarketDataSnapshot, PositionRecord, RiskConfig,
    RiskFinding, RiskReport, RiskRule, RiskScope, RiskStatus, Severity,
};

pub fn recompute_exposure(
    positions: &BTreeMap<String, PositionRecord>,
    market_data: &BTreeMap<String, MarketDataSnapshot>,
    daily_loss: Decimal,
    positions_unknown: bool,
    now: DateTime<Utc>,
) -> ExposureSnapshot {
    let mut by_market: BTreeMap<String, Decimal> = BTreeMap::new();
    let mut verified = !positions_unknown;
    for position in positions.values() {
        let notional = match mark_price(market_data.get(&position.token_id), position.avg_price) {
            Some(px) => position.size.abs() * px,
            None if position.size.is_zero() => Decimal::ZERO,
            None => {
                verified = false;
                Decimal::ZERO
            }
        };
        *by_market
            .entry(position.market_id.clone())
            .or_insert(Decimal::ZERO) += notional;
    }
    let total = by_market.values().copied().sum::<Decimal>();
    ExposureSnapshot {
        by_market,
        total,
        daily_loss,
        verified,
        updated_at: now,
    }
}

fn mark_price(market: Option<&MarketDataSnapshot>, avg: Option<Decimal>) -> Option<Decimal> {
    if let Some(market) = market {
        if let Some(px) = market.last_trade_price {
            return Some(px);
        }
        if let (Some(bid), Some(ask)) = (market.best_bid, market.best_ask) {
            return Some((bid + ask) / Decimal::from(2));
        }
    }
    avg
}

pub fn evaluate(policy: &ControlPolicy, input: &RiskInput<'_>) -> RiskReport {
    let config = &policy.risk;
    let mut findings = Vec::new();
    let now = input.now;

    if input.positions_unknown {
        findings.push(unknown(RiskRule::MaxPosition, RiskScope::Portfolio, now));
        findings.push(unknown(
            RiskRule::MaxMarketExposure,
            RiskScope::Portfolio,
            now,
        ));
        findings.push(unknown(
            RiskRule::MaxTotalExposure,
            RiskScope::Portfolio,
            now,
        ));
    } else {
        for position in input.positions.values() {
            push_limit(
                &mut findings,
                config,
                RiskRule::MaxPosition,
                RiskScope::Token {
                    token_id: position.token_id.clone(),
                },
                position.size.abs(),
                config.max_position,
                now,
            );
        }
        if input.exposure.verified {
            for (market_id, notional) in &input.exposure.by_market {
                push_limit(
                    &mut findings,
                    config,
                    RiskRule::MaxMarketExposure,
                    RiskScope::Market {
                        market_id: market_id.clone(),
                    },
                    *notional,
                    config.max_market_exposure,
                    now,
                );
            }
            push_limit(
                &mut findings,
                config,
                RiskRule::MaxTotalExposure,
                RiskScope::Portfolio,
                input.exposure.total,
                config.max_total_exposure,
                now,
            );
        } else {
            findings.push(unknown(
                RiskRule::MaxMarketExposure,
                RiskScope::Portfolio,
                now,
            ));
            findings.push(unknown(
                RiskRule::MaxTotalExposure,
                RiskScope::Portfolio,
                now,
            ));
        }
    }

    push_limit(
        &mut findings,
        config,
        RiskRule::MaxDailyLoss,
        RiskScope::Portfolio,
        input.daily_loss,
        config.max_daily_loss,
        now,
    );
    push_limit(
        &mut findings,
        config,
        RiskRule::MaxExecutionFailures,
        RiskScope::System,
        Decimal::from(input.execution_failures),
        Decimal::from(config.max_execution_failures),
        now,
    );

    if input.market_data.is_empty() {
        findings.push(unknown(RiskRule::MaxMarketDataAge, RiskScope::System, now));
    } else {
        for snap in input.market_data.values() {
            let scope = RiskScope::Token {
                token_id: snap.token_id.clone(),
            };
            match snap.freshness {
                Freshness::Unknown => {
                    findings.push(unknown(RiskRule::MaxMarketDataAge, scope, now))
                }
                Freshness::Fresh => {}
                Freshness::Stale => {
                    let age = snap
                        .last_event_at
                        .or(snap.last_snapshot_at)
                        .map(|ts| crate::domain::age_ms(now, ts))
                        .unwrap_or(config.max_market_data_age_ms.saturating_add(1));
                    let status = if policy.stale_data_action == StaleDataAction::Degrade {
                        RiskStatus::Warning
                    } else {
                        RiskStatus::Breached
                    };
                    findings.push(finding(
                        config,
                        RiskRule::MaxMarketDataAge,
                        scope,
                        status,
                        Decimal::from(age),
                        Decimal::from(config.max_market_data_age_ms),
                        now,
                    ));
                }
            }
        }
    }

    let status = aggregate(findings.iter().map(|f| f.status));
    RiskReport {
        status,
        findings,
        evaluated_at: now,
    }
}

pub struct RiskInput<'a> {
    pub positions: &'a BTreeMap<String, PositionRecord>,
    pub exposure: &'a ExposureSnapshot,
    pub market_data: &'a BTreeMap<String, MarketDataSnapshot>,
    pub daily_loss: Decimal,
    pub execution_failures: u32,
    pub positions_unknown: bool,
    pub now: DateTime<Utc>,
}

fn push_limit(
    findings: &mut Vec<RiskFinding>,
    config: &RiskConfig,
    rule: RiskRule,
    scope: RiskScope,
    observed: Decimal,
    threshold: Decimal,
    now: DateTime<Utc>,
) {
    let status = classify(observed, threshold, config.warning_ratio);
    if status != RiskStatus::Healthy {
        findings.push(finding(
            config, rule, scope, status, observed, threshold, now,
        ));
    }
}

fn classify(observed: Decimal, threshold: Decimal, warning_ratio: Decimal) -> RiskStatus {
    if threshold < Decimal::ZERO {
        return RiskStatus::Unknown;
    }
    if observed > threshold {
        RiskStatus::Breached
    } else if threshold > Decimal::ZERO && observed > threshold * warning_ratio {
        RiskStatus::Warning
    } else {
        RiskStatus::Healthy
    }
}

fn finding(
    config: &RiskConfig,
    rule: RiskRule,
    scope: RiskScope,
    status: RiskStatus,
    observed: Decimal,
    threshold: Decimal,
    now: DateTime<Utc>,
) -> RiskFinding {
    let action = rule.action(config);
    let severity = match status {
        RiskStatus::Breached if action == BreachAction::Kill => Severity::Critical,
        RiskStatus::Breached | RiskStatus::Unknown => Severity::Critical,
        RiskStatus::Warning => Severity::Warning,
        RiskStatus::Healthy => Severity::Info,
    };
    RiskFinding {
        rule,
        status,
        scope,
        observed,
        threshold,
        at: now,
        severity,
        action,
    }
}

fn unknown(rule: RiskRule, scope: RiskScope, now: DateTime<Utc>) -> RiskFinding {
    RiskFinding {
        rule,
        status: RiskStatus::Unknown,
        scope,
        observed: Decimal::ZERO,
        threshold: Decimal::ZERO,
        at: now,
        severity: Severity::Critical,
        action: BreachAction::Pause,
    }
}

fn aggregate(statuses: impl Iterator<Item = RiskStatus>) -> RiskStatus {
    let mut saw_warning = false;
    let mut saw_unknown = false;
    let mut saw_any = false;
    for status in statuses {
        saw_any = true;
        match status {
            RiskStatus::Breached => return RiskStatus::Breached,
            RiskStatus::Unknown => saw_unknown = true,
            RiskStatus::Warning => saw_warning = true,
            RiskStatus::Healthy => {}
        }
    }
    if !saw_any {
        RiskStatus::Healthy
    } else if saw_unknown {
        RiskStatus::Unknown
    } else if saw_warning {
        RiskStatus::Warning
    } else {
        RiskStatus::Healthy
    }
}
