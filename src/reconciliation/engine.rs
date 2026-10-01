use std::collections::{BTreeMap, BTreeSet};

use chrono::{DateTime, Utc};
use rust_decimal::Decimal;

use crate::domain::{
    EntityType, ExecutionVerdict, FillRecord, Mismatch, OrderRecord, PositionRecord,
    ReconciliationReport, ReconciliationStatus, Severity,
};

pub struct ReconcileBooks<'a> {
    pub orders_local: &'a BTreeMap<String, OrderRecord>,
    pub orders_remote: &'a BTreeMap<String, OrderRecord>,
    pub fills_local: &'a BTreeMap<String, FillRecord>,
    pub fills_remote: &'a BTreeMap<String, FillRecord>,
    pub positions_local: &'a BTreeMap<String, PositionRecord>,
    pub positions_remote: &'a BTreeMap<String, PositionRecord>,
    pub positions_unknown: bool,
    pub execution: ExecutionVerdict,
    pub now: DateTime<Utc>,
}

pub fn reconcile(books: ReconcileBooks<'_>) -> ReconciliationReport {
    let (orders, order_mismatches) = compare_numeric(
        books.orders_local,
        books.orders_remote,
        |o| o.size_matched,
        EntityType::Order,
        books.now,
    );
    let (fills, fill_mismatches) = compare_numeric(
        books.fills_local,
        books.fills_remote,
        |f| f.size,
        EntityType::Fill,
        books.now,
    );
    let (positions, position_mismatches) = if books.positions_unknown {
        (ReconciliationStatus::Unknown, Vec::new())
    } else {
        compare_numeric(
            books.positions_local,
            books.positions_remote,
            |p| p.size,
            EntityType::Position,
            books.now,
        )
    };
    let execution_status = match books.execution {
        ExecutionVerdict::Verified => ReconciliationStatus::Match,
        ExecutionVerdict::Inconsistent => ReconciliationStatus::Mismatch,
        ExecutionVerdict::Unknown | ExecutionVerdict::NotChecked => ReconciliationStatus::Unknown,
    };
    let mut mismatches = order_mismatches;
    mismatches.extend(fill_mismatches);
    mismatches.extend(position_mismatches);
    if books.execution == ExecutionVerdict::Inconsistent {
        mismatches.push(Mismatch {
            entity_type: EntityType::Execution,
            entity_id: "execution".to_string(),
            expected: Decimal::ONE,
            observed: Decimal::ZERO,
            difference: -Decimal::ONE,
            at: books.now,
            source: "execution-verifier".to_string(),
            reason: "execution verdict is INCONSISTENT".to_string(),
            severity: Severity::Critical,
        });
    }
    let status = fold(&[orders, fills, positions, execution_status]);
    ReconciliationReport {
        status,
        mismatches,
        orders,
        fills,
        positions,
        execution: execution_status,
        at: books.now,
    }
}

fn compare_numeric<T>(
    local: &BTreeMap<String, T>,
    remote: &BTreeMap<String, T>,
    value: impl Fn(&T) -> Decimal,
    entity_type: EntityType,
    now: DateTime<Utc>,
) -> (ReconciliationStatus, Vec<Mismatch>) {
    let mut ids = BTreeSet::new();
    ids.extend(local.keys().cloned());
    ids.extend(remote.keys().cloned());
    let mut mismatches = Vec::new();
    for id in ids {
        let expected = local.get(&id).map(&value).unwrap_or(Decimal::ZERO);
        let observed = remote.get(&id).map(&value).unwrap_or(Decimal::ZERO);
        if expected == observed {
            continue;
        }
        mismatches.push(Mismatch {
            entity_type,
            entity_id: id,
            expected,
            observed,
            difference: observed - expected,
            at: now,
            source: "local-vs-remote".to_string(),
            reason: format!("{entity_type} values differ"),
            severity: Severity::Critical,
        });
    }
    let status = if mismatches.is_empty() {
        ReconciliationStatus::Match
    } else {
        ReconciliationStatus::Mismatch
    };
    (status, mismatches)
}

fn fold(parts: &[ReconciliationStatus]) -> ReconciliationStatus {
    if parts.contains(&ReconciliationStatus::Failed) {
        ReconciliationStatus::Failed
    } else if parts.contains(&ReconciliationStatus::Mismatch) {
        ReconciliationStatus::Mismatch
    } else if parts.contains(&ReconciliationStatus::Unknown) {
        ReconciliationStatus::Unknown
    } else {
        ReconciliationStatus::Match
    }
}

/// Upgrade a matching report after a recovery run that actually completed.
pub fn mark_recovered(report: &mut ReconciliationReport) {
    if report.status == ReconciliationStatus::Match {
        report.status = ReconciliationStatus::Recovered;
        if report.orders == ReconciliationStatus::Match {
            report.orders = ReconciliationStatus::Recovered;
        }
        if report.fills == ReconciliationStatus::Match {
            report.fills = ReconciliationStatus::Recovered;
        }
        if report.positions == ReconciliationStatus::Match {
            report.positions = ReconciliationStatus::Recovered;
        }
        if report.execution == ReconciliationStatus::Match {
            report.execution = ReconciliationStatus::Recovered;
        }
    }
}
