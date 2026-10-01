use serde::{Deserialize, Serialize};

use crate::domain::RiskConfig;

/// What stale market data does to the system state.
///
/// Trading permission is separate: stale data blocks trading unless
/// [`ControlPolicy::stale_blocks_trading`] is turned off.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum StaleDataAction {
    Pause,
    Degrade,
}

/// Policy inputs. Limits live in [`RiskConfig`]; this struct decides how
/// warnings and staleness affect permission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ControlPolicy {
    pub risk: RiskConfig,
    pub stale_data_action: StaleDataAction,
    pub stale_blocks_trading: bool,
    /// When false, `DEGRADED` still blocks new trading.
    pub degraded_allows_trading: bool,
    pub require_execution_verification: bool,
}

impl Default for ControlPolicy {
    fn default() -> Self {
        Self {
            risk: RiskConfig::default(),
            stale_data_action: StaleDataAction::Pause,
            stale_blocks_trading: true,
            degraded_allows_trading: false,
            require_execution_verification: true,
        }
    }
}
