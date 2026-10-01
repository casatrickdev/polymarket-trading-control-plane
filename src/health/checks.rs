use std::collections::BTreeMap;

use chrono::{DateTime, Utc};

use crate::control::ControlPolicy;
use crate::domain::{age_ms, ConnectionHealth, ConnectionSnapshot, Freshness, MarketDataSnapshot};

pub fn market_freshness(
    snap: &MarketDataSnapshot,
    now: DateTime<Utc>,
    max_age_ms: u64,
) -> (Freshness, Option<u64>) {
    let stamp = snap.last_event_at.or(snap.last_snapshot_at);
    match stamp {
        None => (Freshness::Unknown, None),
        Some(then) => {
            let age = age_ms(now, then);
            if age > max_age_ms {
                (Freshness::Stale, Some(age))
            } else {
                (Freshness::Fresh, Some(age))
            }
        }
    }
}

/// `Connected` stays the reported transport state. This returns the effective
/// health, which is `Stale` when the socket is up but quiet or old.
pub fn effective_connection(
    conn: &ConnectionSnapshot,
    now: DateTime<Utc>,
    max_age_ms: u64,
) -> ConnectionHealth {
    match conn.reported {
        ConnectionHealth::Failed => ConnectionHealth::Failed,
        ConnectionHealth::Disconnected => ConnectionHealth::Disconnected,
        ConnectionHealth::Reconnecting => ConnectionHealth::Reconnecting,
        ConnectionHealth::Stale => ConnectionHealth::Stale,
        ConnectionHealth::Connected => match conn.last_event_at {
            None => ConnectionHealth::Stale,
            Some(then) if age_ms(now, then) > max_age_ms => ConnectionHealth::Stale,
            Some(_) => ConnectionHealth::Connected,
        },
    }
}

pub fn refresh_market_data(
    markets: &mut BTreeMap<String, MarketDataSnapshot>,
    now: DateTime<Utc>,
    policy: &ControlPolicy,
) {
    let max_age = policy.risk.max_market_data_age_ms;
    for snap in markets.values_mut() {
        snap.freshness = market_freshness(snap, now, max_age).0;
    }
}

pub fn refresh_connections(
    connections: &mut BTreeMap<String, ConnectionSnapshot>,
    now: DateTime<Utc>,
    policy: &ControlPolicy,
) {
    let max_age = policy.risk.max_connection_event_age_ms;
    for conn in connections.values_mut() {
        conn.effective = effective_connection(conn, now, max_age);
        conn.updated_at = now;
    }
}
