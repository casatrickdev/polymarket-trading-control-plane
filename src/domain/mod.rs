//! Domain model for the trading control plane.
//!
//! These types are independent of Polymarket transport. Adapters normalize
//! external payloads into [`events::DomainEvent`] before they reach the engines.

mod alerts;
mod events;
mod health;
mod reconciliation;
mod records;
mod recovery;
mod risk;
mod system_state;
mod trading_permission;

pub use alerts::*;
pub use events::*;
pub use health::*;
pub use reconciliation::*;
pub use records::*;
pub use recovery::*;
pub use risk::*;
pub use system_state::*;
pub use trading_permission::*;
