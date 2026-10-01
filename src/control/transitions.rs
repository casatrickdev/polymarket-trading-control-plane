use crate::domain::{SystemState, TransitionContext, TransitionKind, TransitionReason};
use crate::error::ControlError;

/// Validates a transition against the matrix and the policy context.
///
/// Identical states are idempotent. `Killed` has no exit. Resume and recovery
/// completion require open gates; a websocket reconnect is not one of those kinds.
pub fn transition_allowed(
    from: SystemState,
    to: SystemState,
    kind: TransitionKind,
    ctx: &TransitionContext,
) -> bool {
    if ctx.kill_latched || from == SystemState::Killed {
        return to == SystemState::Killed && (kind == TransitionKind::Kill || from == to);
    }
    if from == to {
        return match kind {
            TransitionKind::Resume => ctx.gates_open && !ctx.recovery_required,
            _ => true,
        };
    }
    match (from, to, kind) {
        (_, SystemState::Killed, TransitionKind::Kill) => true,
        (
            SystemState::Healthy
            | SystemState::Degraded
            | SystemState::Recovering
            | SystemState::Failed,
            SystemState::Paused,
            TransitionKind::Pause | TransitionKind::Automatic,
        ) => true,
        (SystemState::Paused, SystemState::Recovering, TransitionKind::RecoveryStart) => true,
        (
            SystemState::Healthy | SystemState::Degraded | SystemState::Failed,
            SystemState::Recovering,
            TransitionKind::RecoveryStart,
        ) => true,
        (
            SystemState::Paused,
            SystemState::Healthy,
            TransitionKind::Resume | TransitionKind::Automatic,
        ) => {
            ctx.gates_open && !ctx.operator_paused && !ctx.recovery_required && !ctx.recovery_active
        }
        (SystemState::Paused, SystemState::Degraded, TransitionKind::Automatic) => {
            !ctx.operator_paused && !ctx.recovery_required && !ctx.recovery_active
        }
        (SystemState::Paused, SystemState::Degraded, TransitionKind::Resume) => {
            ctx.gates_open && !ctx.recovery_required && !ctx.recovery_active
        }
        (SystemState::Recovering, SystemState::Healthy, TransitionKind::RecoveryComplete) => {
            ctx.gates_open && !ctx.operator_paused && !ctx.recovery_required
        }
        (SystemState::Recovering, SystemState::Degraded, TransitionKind::RecoveryComplete) => {
            !ctx.operator_paused && !ctx.recovery_required
        }
        (SystemState::Recovering, SystemState::Paused, TransitionKind::RecoveryComplete) => true,
        (SystemState::Recovering, SystemState::Failed, TransitionKind::Automatic) => true,
        (SystemState::Healthy, SystemState::Degraded, TransitionKind::Automatic) => true,
        (SystemState::Healthy, SystemState::Failed, TransitionKind::Automatic) => true,
        (SystemState::Paused, SystemState::Failed, TransitionKind::Automatic) => true,
        (SystemState::Degraded, SystemState::Healthy, TransitionKind::Automatic) => {
            ctx.gates_open && !ctx.recovery_required && !ctx.operator_paused
        }
        (SystemState::Degraded, SystemState::Failed, TransitionKind::Automatic) => true,
        _ => false,
    }
}

pub fn apply_transition(
    from: SystemState,
    to: SystemState,
    kind: TransitionKind,
    reason: TransitionReason,
    ctx: &TransitionContext,
) -> Result<SystemState, ControlError> {
    if transition_allowed(from, to, kind, ctx) {
        Ok(to)
    } else if from == SystemState::Killed || ctx.kill_latched {
        Err(ControlError::BlockedByKill)
    } else {
        Err(ControlError::InvalidTransition { from, to, reason })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(gates: bool) -> TransitionContext {
        TransitionContext {
            gates_open: gates,
            ..TransitionContext::default()
        }
    }

    #[test]
    fn table_driven_transitions() {
        let cases = [
            (
                SystemState::Healthy,
                SystemState::Degraded,
                TransitionKind::Automatic,
                true,
                true,
            ),
            (
                SystemState::Degraded,
                SystemState::Paused,
                TransitionKind::Automatic,
                false,
                true,
            ),
            (
                SystemState::Paused,
                SystemState::Recovering,
                TransitionKind::RecoveryStart,
                false,
                true,
            ),
            (
                SystemState::Recovering,
                SystemState::Healthy,
                TransitionKind::RecoveryComplete,
                true,
                true,
            ),
            (
                SystemState::Recovering,
                SystemState::Paused,
                TransitionKind::RecoveryComplete,
                false,
                true,
            ),
            (
                SystemState::Healthy,
                SystemState::Failed,
                TransitionKind::Automatic,
                true,
                true,
            ),
            (
                SystemState::Paused,
                SystemState::Healthy,
                TransitionKind::Resume,
                true,
                true,
            ),
            (
                SystemState::Paused,
                SystemState::Healthy,
                TransitionKind::Resume,
                false,
                false,
            ),
            (
                SystemState::Failed,
                SystemState::Healthy,
                TransitionKind::Automatic,
                true,
                false,
            ),
            (
                SystemState::Killed,
                SystemState::Healthy,
                TransitionKind::Resume,
                true,
                false,
            ),
            (
                SystemState::Healthy,
                SystemState::Killed,
                TransitionKind::Kill,
                false,
                true,
            ),
            (
                SystemState::Paused,
                SystemState::Paused,
                TransitionKind::Pause,
                false,
                true,
            ),
        ];
        for (from, to, kind, gates, expect) in cases {
            let allowed = transition_allowed(from, to, kind, &ctx(gates));
            assert_eq!(allowed, expect, "{from} -> {to} via {kind:?} gates={gates}");
        }
    }

    #[test]
    fn resume_blocked_while_recovery_required() {
        let mut context = ctx(true);
        context.recovery_required = true;
        assert!(!transition_allowed(
            SystemState::Paused,
            SystemState::Healthy,
            TransitionKind::Resume,
            &context
        ));
    }

    #[test]
    fn operator_pause_blocks_automatic_resume() {
        let mut context = ctx(true);
        context.operator_paused = true;
        assert!(!transition_allowed(
            SystemState::Paused,
            SystemState::Healthy,
            TransitionKind::Automatic,
            &context
        ));
    }
}
