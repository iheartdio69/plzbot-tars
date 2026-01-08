use crate::config::Config;
use crate::types::{CallRecord, CoinState};
use std::collections::HashMap;

// Placeholder: later we’ll compare price now vs call price at +5m/+15m etc.
pub fn resolve_calls(
    _cfg: &Config,
    _coins: &HashMap<String, CoinState>,
    _calls: &mut Vec<CallRecord>,
) {
    // keep empty for now
}
