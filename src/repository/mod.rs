mod failing;
mod memory;
mod sqlite;

use crate::domain::{Alert, StateTransition};
use crate::error::ControlError;
use crate::runtime::RuntimeState;

pub use failing::FailingStore;
pub use memory::MemoryStore;
pub use sqlite::SqliteStore;

pub trait StateStore: Send {
    fn load(&self) -> Result<Option<RuntimeState>, ControlError>;
    fn save(&self, state: &RuntimeState) -> Result<(), ControlError>;
    fn transitions(&self) -> Result<Vec<StateTransition>, ControlError>;
    fn alerts(&self) -> Result<Vec<Alert>, ControlError>;
}
