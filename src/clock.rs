use std::sync::Mutex;

use chrono::{DateTime, Utc};

use crate::error::ControlError;

/// Clock used for freshness. Tests pin it; `refresh` reads the wall clock.
#[derive(Debug)]
pub struct Clock {
    manual: Mutex<Option<DateTime<Utc>>>,
}

impl Clock {
    pub fn manual(now: DateTime<Utc>) -> Self {
        Self {
            manual: Mutex::new(Some(now)),
        }
    }

    pub fn system() -> Self {
        Self {
            manual: Mutex::new(None),
        }
    }

    pub fn now(&self) -> Result<DateTime<Utc>, ControlError> {
        let guard = self.manual.lock().map_err(|_| ControlError::LockPoisoned)?;
        Ok(guard.unwrap_or_else(Utc::now))
    }

    pub fn set(&self, now: DateTime<Utc>) -> Result<(), ControlError> {
        let mut guard = self.manual.lock().map_err(|_| ControlError::LockPoisoned)?;
        *guard = Some(now);
        Ok(())
    }

    pub fn refresh(&self) -> Result<DateTime<Utc>, ControlError> {
        let now = Utc::now();
        self.set(now)?;
        Ok(now)
    }
}
