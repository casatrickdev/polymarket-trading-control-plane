//! Polymarket trading control plane.
//!
//! The strategy decides what to trade. This crate decides whether the system
//! is healthy enough to keep trading. It does not place orders and it does
//! not hold private keys.

pub mod adapters;
pub mod alerts;
pub mod clock;
pub mod control;
pub mod demo;
pub mod domain;
pub mod error;
pub mod health;
pub mod reconciliation;
pub mod recovery;
pub mod repository;
pub mod risk;
pub mod runtime;
pub mod telemetry;

pub use control::{ControlPlane, ControlPolicy, Evaluation};
pub use domain::{SystemState, TradeDecision, TradingPermission};
pub use error::ControlError;
