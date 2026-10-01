use crate::domain::{SystemState, TransitionReason};

/// Structured failures returned by the control plane.
///
/// Library paths return these values. They do not panic on operational uncertainty.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ControlError {
    #[error("invalid transition from {from} to {to} because {reason}")]
    InvalidTransition {
        from: SystemState,
        to: SystemState,
        reason: TransitionReason,
    },
    #[error("resume blocked: {reasons}")]
    ResumeBlocked { reasons: String },
    #[error("kill switch is latched; resume cannot clear it")]
    BlockedByKill,
    #[error("persistence error: {0}")]
    Persistence(String),
    #[error("authoritative source error: {0}")]
    Source(String),
    #[error("internal lock poisoned")]
    LockPoisoned,
    #[error("{0}")]
    Message(String),
}
