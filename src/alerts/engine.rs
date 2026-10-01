use chrono::{DateTime, Utc};
use serde_json::{json, Value};
use uuid::Uuid;

use crate::domain::{Alert, AlertType, HealthReason, Severity, SystemState, TradingPermission};

use super::dedup;

pub struct AlertDraft {
    pub event_type: AlertType,
    pub severity: Severity,
    pub scope: String,
    pub entity_id: Option<String>,
    pub message: String,
    pub metadata: Value,
}

pub fn raise_condition(alerts: &mut Vec<Alert>, draft: AlertDraft, now: DateTime<Utc>) -> bool {
    let key = Alert::condition_key(draft.event_type, &draft.scope, draft.entity_id.as_deref());
    if let Some(existing) = dedup::unresolved_mut(alerts, &key) {
        existing.severity = draft.severity;
        existing.message = draft.message;
        existing.metadata = draft.metadata;
        return false;
    }
    alerts.push(Alert {
        id: Uuid::new_v4().to_string(),
        event_type: draft.event_type,
        severity: draft.severity,
        at: now,
        scope: draft.scope,
        entity_id: draft.entity_id,
        message: draft.message,
        metadata: draft.metadata,
        resolved: false,
        resolved_at: None,
        dedup_key: key,
    });
    true
}

pub fn resolve_condition(alerts: &mut [Alert], key: &str, now: DateTime<Utc>) {
    if let Some(alert) = dedup::unresolved_mut(alerts, key) {
        alert.resolved = true;
        alert.resolved_at = Some(now);
    }
}

/// Point-in-time alerts are not deduplicated. They do not stay critical blockers
/// unless the caller asks for a condition via [`raise_condition`].
pub fn push_event(alerts: &mut Vec<Alert>, draft: AlertDraft, now: DateTime<Utc>) {
    let key = format!(
        "{}:{}:{}:{}",
        draft.event_type.as_str(),
        draft.scope,
        draft.entity_id.as_deref().unwrap_or("-"),
        now.timestamp_millis()
    );
    alerts.push(Alert {
        id: Uuid::new_v4().to_string(),
        event_type: draft.event_type,
        severity: draft.severity,
        at: now,
        scope: draft.scope,
        entity_id: draft.entity_id,
        message: draft.message,
        metadata: draft.metadata,
        resolved: false,
        resolved_at: None,
        dedup_key: key,
    });
}

pub fn sync_conditions(
    alerts: &mut Vec<Alert>,
    reasons: &[HealthReason],
    state: SystemState,
    permission: TradingPermission,
    now: DateTime<Utc>,
) {
    let mut desired: Vec<AlertDraft> = Vec::new();
    for reason in reasons {
        if let Some(draft) = draft_for_reason(reason) {
            desired.push(draft);
        }
    }
    if permission != TradingPermission::TradingAllowed && !matches!(state, SystemState::Healthy) {
        desired.push(AlertDraft {
            event_type: AlertType::TradingPaused,
            severity: if matches!(state, SystemState::Killed | SystemState::Failed) {
                Severity::Critical
            } else {
                Severity::Warning
            },
            scope: "system".to_string(),
            entity_id: None,
            message: format!("trading is not allowed while system is {state}"),
            metadata: json!({ "system_state": state.as_str(), "permission": permission.as_str() }),
        });
    }
    if state == SystemState::Killed {
        desired.push(AlertDraft {
            event_type: AlertType::KillSwitchTriggered,
            severity: Severity::Critical,
            scope: "system".to_string(),
            entity_id: None,
            message: "kill switch is latched".to_string(),
            metadata: json!({ "system_state": "KILLED" }),
        });
    }

    let desired_keys: Vec<String> = desired
        .iter()
        .map(|d| Alert::condition_key(d.event_type, &d.scope, d.entity_id.as_deref()))
        .collect();

    for mut draft in desired {
        if draft.event_type == AlertType::MarketDataStale
            && matches!(
                state,
                SystemState::Paused | SystemState::Failed | SystemState::Killed
            )
        {
            draft.severity = Severity::Critical;
        }
        raise_condition(alerts, draft, now);
    }

    let managed = [
        AlertType::StateMismatch,
        AlertType::MarketDataStale,
        AlertType::WebsocketDisconnected,
        AlertType::RiskLimitBreached,
        AlertType::TradingPaused,
        AlertType::KillSwitchTriggered,
    ];
    let stale_keys: Vec<String> = alerts
        .iter()
        .filter(|a| {
            !a.resolved && managed.contains(&a.event_type) && !desired_keys.contains(&a.dedup_key)
        })
        .map(|a| a.dedup_key.clone())
        .collect();
    for key in stale_keys {
        resolve_condition(alerts, &key, now);
    }
}

fn draft_for_reason(reason: &HealthReason) -> Option<AlertDraft> {
    match reason {
        HealthReason::PositionMismatch { token_id } => Some(AlertDraft {
            event_type: AlertType::StateMismatch,
            severity: Severity::Critical,
            scope: "position".to_string(),
            entity_id: Some(token_id.clone()),
            message: reason.to_string(),
            metadata: json!({ "reason": reason.to_string() }),
        }),
        HealthReason::ReconciliationMismatch => Some(AlertDraft {
            event_type: AlertType::StateMismatch,
            severity: Severity::Critical,
            scope: "position".to_string(),
            entity_id: None,
            message: reason.to_string(),
            metadata: json!({ "reason": reason.to_string() }),
        }),
        HealthReason::MarketDataStale {
            token_id,
            age_ms,
            max_age_ms,
        } => Some(AlertDraft {
            event_type: AlertType::MarketDataStale,
            severity: Severity::Warning,
            scope: "market-data".to_string(),
            entity_id: Some(token_id.clone()),
            message: reason.to_string(),
            metadata: json!({ "age_ms": age_ms, "max_age_ms": max_age_ms }),
        }),
        HealthReason::ConnectionDisconnected => Some(AlertDraft {
            event_type: AlertType::WebsocketDisconnected,
            severity: Severity::Critical,
            scope: "connection".to_string(),
            entity_id: None,
            message: reason.to_string(),
            metadata: json!({}),
        }),
        HealthReason::RiskBreached { rule } => Some(AlertDraft {
            event_type: AlertType::RiskLimitBreached,
            severity: Severity::Critical,
            scope: "risk".to_string(),
            entity_id: Some(rule.clone()),
            message: reason.to_string(),
            metadata: json!({ "rule": rule }),
        }),
        HealthReason::KillSwitchActive => Some(AlertDraft {
            event_type: AlertType::KillSwitchTriggered,
            severity: Severity::Critical,
            scope: "system".to_string(),
            entity_id: None,
            message: "kill switch is latched".to_string(),
            metadata: json!({}),
        }),
        _ => None,
    }
}
