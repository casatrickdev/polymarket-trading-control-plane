use std::sync::Mutex;

use crate::domain::{Alert, StateTransition};
use crate::error::ControlError;
use crate::runtime::RuntimeState;

use super::StateStore;

#[derive(Debug, Default)]
pub struct MemoryStore {
    inner: Mutex<Option<RuntimeState>>,
}

impl MemoryStore {
    pub fn new() -> Self {
        Self::default()
    }
}

impl StateStore for MemoryStore {
    fn load(&self) -> Result<Option<RuntimeState>, ControlError> {
        let guard = self.inner.lock().map_err(|_| ControlError::LockPoisoned)?;
        Ok(guard.clone())
    }

    fn save(&self, state: &RuntimeState) -> Result<(), ControlError> {
        let mut guard = self.inner.lock().map_err(|_| ControlError::LockPoisoned)?;
        *guard = Some(state.clone());
        Ok(())
    }

    fn transitions(&self) -> Result<Vec<StateTransition>, ControlError> {
        let guard = self.inner.lock().map_err(|_| ControlError::LockPoisoned)?;
        Ok(guard
            .as_ref()
            .map(|state| state.transitions.clone())
            .unwrap_or_default())
    }

    fn alerts(&self) -> Result<Vec<Alert>, ControlError> {
        let guard = self.inner.lock().map_err(|_| ControlError::LockPoisoned)?;
        Ok(guard
            .as_ref()
            .map(|state| state.alerts.clone())
            .unwrap_or_default())
    }
}
