use std::sync::atomic::{AtomicBool, Ordering};

use crate::domain::{Alert, StateTransition};
use crate::error::ControlError;
use crate::runtime::RuntimeState;

use super::StateStore;

/// Test double that can fail closed on writes.
pub struct FailingStore<S> {
    inner: S,
    fail_writes: AtomicBool,
}

impl<S> FailingStore<S> {
    pub fn new(inner: S) -> Self {
        Self {
            inner,
            fail_writes: AtomicBool::new(false),
        }
    }

    pub fn set_fail_writes(&self, fail: bool) {
        self.fail_writes.store(fail, Ordering::SeqCst);
    }
}

impl<S: StateStore> StateStore for FailingStore<S> {
    fn load(&self) -> Result<Option<RuntimeState>, ControlError> {
        self.inner.load()
    }

    fn save(&self, state: &RuntimeState) -> Result<(), ControlError> {
        if self.fail_writes.load(Ordering::SeqCst) {
            return Err(ControlError::Persistence(
                "injected persistence failure".to_string(),
            ));
        }
        self.inner.save(state)
    }

    fn transitions(&self) -> Result<Vec<StateTransition>, ControlError> {
        self.inner.transitions()
    }

    fn alerts(&self) -> Result<Vec<Alert>, ControlError> {
        self.inner.alerts()
    }
}
