use crate::config::Config;
use crate::market::discovery::{MarketDiscovery};
use crate::types::CoinState;
use std::collections::HashMap;
use std::time::Instant;

/// Seed `coins` with market-discovered mints so your scorer can see real runners.
/// This DOES NOT mark them active or call them; it simply ensures they exist in tracking.
pub async fn market_discovery_seed(
    cfg: &Config,
    discovery: &mut MarketDiscovery,
    coins: &mut HashMap<String, CoinState>,
) {
    if !discovery.should_run(cfg) {
        return;
    }

    let mints = discovery.run(cfg).await;

    let mut added = 0usize;
    for mint in mints {
        if coins.contains_key(&mint) {
            continue;
        }
        // Minimal CoinState init—uses your existing fields from earlier work.
        coins.insert(
            mint.clone(),
            CoinState {
                mint,
                events: Vec::new(),
                active: false,
                low_score_streak: 0,
                first_seen: Instant::now(),
                last_snapshot: Instant::now(),
                prev_tx_window: 0,
                prev_signers_window: 0,
            },
        );
        added += 1;
    }

    if added > 0 {
        println!("🧲 market discovery seeded {} new mints into tracking", added);
    }
}
