//! External data-source boundaries.
//!
//! The domain evaluates normalized events. Mock sources back tests and the
//! local demo. The Polymarket adapter is compiled with `--features polymarket`.

mod mock;
mod traits;

#[cfg(feature = "polymarket")]
pub mod polymarket;

pub use mock::*;
pub use traits::*;
