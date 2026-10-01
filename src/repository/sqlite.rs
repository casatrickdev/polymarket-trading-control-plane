use std::path::Path;
use std::sync::Mutex;

use chrono::Utc;
use rusqlite::{params, Connection};

use crate::domain::{Alert, StateTransition};
use crate::error::ControlError;
use crate::runtime::RuntimeState;

use super::StateStore;

pub struct SqliteStore {
    conn: Mutex<Connection>,
}

impl SqliteStore {
    pub fn open(path: impl AsRef<Path>) -> Result<Self, ControlError> {
        let conn = Connection::open(path.as_ref())
            .map_err(|err| ControlError::Persistence(err.to_string()))?;
        conn.execute_batch(include_str!("../../migrations/001_init.sql"))
            .map_err(|err| ControlError::Persistence(err.to_string()))?;
        conn.execute(
            "INSERT OR IGNORE INTO schema_migrations (version, applied_at) VALUES (1, ?1)",
            params![Utc::now().to_rfc3339()],
        )
        .map_err(|err| ControlError::Persistence(err.to_string()))?;
        Ok(Self {
            conn: Mutex::new(conn),
        })
    }
}

impl StateStore for SqliteStore {
    fn load(&self) -> Result<Option<RuntimeState>, ControlError> {
        let conn = self.conn.lock().map_err(|_| ControlError::LockPoisoned)?;
        let mut stmt = conn
            .prepare("SELECT payload FROM control_snapshot WHERE id = 1")
            .map_err(|err| ControlError::Persistence(err.to_string()))?;
        let mut rows = stmt
            .query([])
            .map_err(|err| ControlError::Persistence(err.to_string()))?;
        let Some(row) = rows
            .next()
            .map_err(|err| ControlError::Persistence(err.to_string()))?
        else {
            return Ok(None);
        };
        let payload: String = row
            .get(0)
            .map_err(|err| ControlError::Persistence(err.to_string()))?;
        let state = serde_json::from_str(&payload)
            .map_err(|err| ControlError::Persistence(err.to_string()))?;
        Ok(Some(state))
    }

    fn save(&self, state: &RuntimeState) -> Result<(), ControlError> {
        let mut conn = self.conn.lock().map_err(|_| ControlError::LockPoisoned)?;
        let tx = conn
            .transaction()
            .map_err(|err| ControlError::Persistence(err.to_string()))?;
        let payload = serde_json::to_string(state)
            .map_err(|err| ControlError::Persistence(err.to_string()))?;
        tx.execute(
            "INSERT INTO control_snapshot (id, payload, updated_at) VALUES (1, ?1, ?2)
             ON CONFLICT(id) DO UPDATE SET payload = excluded.payload, updated_at = excluded.updated_at",
            params![payload, state.updated_at.to_rfc3339()],
        )
        .map_err(|err| ControlError::Persistence(err.to_string()))?;
        tx.execute("DELETE FROM state_transitions", [])
            .map_err(|err| ControlError::Persistence(err.to_string()))?;
        for transition in &state.transitions {
            tx.execute(
                "INSERT INTO state_transitions (previous, next, reason, actor, at, detail)
                 VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![
                    transition.previous.as_str(),
                    transition.next.as_str(),
                    transition.reason.to_string(),
                    transition.actor.to_string(),
                    transition.at.to_rfc3339(),
                    transition.detail,
                ],
            )
            .map_err(|err| ControlError::Persistence(err.to_string()))?;
        }
        tx.execute("DELETE FROM alerts", [])
            .map_err(|err| ControlError::Persistence(err.to_string()))?;
        for alert in &state.alerts {
            let payload = serde_json::to_string(alert)
                .map_err(|err| ControlError::Persistence(err.to_string()))?;
            tx.execute(
                "INSERT INTO alerts (id, dedup_key, resolved, payload, updated_at)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
                params![
                    alert.id,
                    alert.dedup_key,
                    alert.resolved as i64,
                    payload,
                    alert.at.to_rfc3339(),
                ],
            )
            .map_err(|err| ControlError::Persistence(err.to_string()))?;
        }
        tx.commit()
            .map_err(|err| ControlError::Persistence(err.to_string()))?;
        Ok(())
    }

    fn transitions(&self) -> Result<Vec<StateTransition>, ControlError> {
        Ok(self
            .load()?
            .map(|state| state.transitions)
            .unwrap_or_default())
    }

    fn alerts(&self) -> Result<Vec<Alert>, ControlError> {
        Ok(self.load()?.map(|state| state.alerts).unwrap_or_default())
    }
}
