use crate::domain::StateTransition;

/// Structured transition log. Callers must not pass secrets into `detail`.
pub fn log_transition(transition: &StateTransition) {
    tracing::info!(
        event_type = "SYSTEM_STATE_CHANGED",
        timestamp = %transition.at,
        system_state = %transition.next,
        from = %transition.previous,
        to = %transition.next,
        reason = %transition.reason,
        actor = %transition.actor,
        kind = ?transition.kind,
        detail = %transition.detail,
        "system state changed"
    );
}

pub fn log_permission(state: &crate::runtime::RuntimeState) {
    tracing::info!(
        event_type = "TRADING_PERMISSION",
        timestamp = %state.updated_at,
        system_state = %state.system_state,
        trading_permission = %state.permission,
        decision = %state.decision,
        "trading permission evaluated"
    );
}
