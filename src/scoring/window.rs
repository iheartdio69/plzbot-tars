// src/scoring/window.rs
use crate::types::{CoinState, Event, WhaleTier};
use std::collections::HashSet;

/// Update rolling 5-minute windows for a coin using st.events.
///
/// - unique_signers_5m = distinct wallets seen in last 300s
/// - tx_5m = number of events in last 300s
pub fn update_windows(st: &mut CoinState, now: u64) {
    let cutoff = now.saturating_sub(300);

    // unique signers in last 5m
    let mut signers: HashSet<&str> = HashSet::new();
    for ev in st.events.iter().rev() {
        if ev.ts < cutoff {
            break;
        }
        signers.insert(ev.wallet.as_str());
    }
    st.unique_signers_5m = signers.len();

    // tx_5m from event count in last 5m
    let mut tx_5m = 0usize;
    for ev in st.events.iter().rev() {
        if ev.ts < cutoff {
            break;
        }
        tx_5m += 1;
    }
    st.tx_5m = tx_5m;
}

#[derive(Debug, Default)]
pub struct WhaleCounts {
    pub beluga_count: usize,
    pub blue_count: usize,
}

/// Prune old events older than keep_secs.
pub fn prune_events(events: &mut Vec<Event>, keep_secs: u64) {
    let now: u64 = crate::time::now() as u64;
    events.retain(|e| now.saturating_sub(e.ts) < keep_secs);
}

/// Returns (tx_count, unique_wallets, whale_counts) for a given window.
pub fn window_stats_for(events: &Vec<Event>, window_secs: u64) -> (usize, usize, WhaleCounts) {
    let now: u64 = crate::time::now() as u64;

    let recent: Vec<&Event> = events
        .iter()
        .filter(|e| now.saturating_sub(e.ts) < window_secs)
        .collect();

    let tx_count = recent.len();

    let unique_wallets = recent
        .iter()
        .map(|e| e.wallet.as_str())
        .collect::<HashSet<&str>>()
        .len();

    let beluga = recent
        .iter()
        .filter(|e| e.tier == WhaleTier::Beluga)
        .count();
    let blue = recent.iter().filter(|e| e.tier == WhaleTier::Blue).count();

    (
        tx_count,
        unique_wallets,
        WhaleCounts {
            beluga_count: beluga,
            blue_count: blue,
        },
    )
}

/// Unique wallets seen in the last window_secs.
pub fn window_wallets(events: &Vec<Event>, window_secs: u64) -> Vec<String> {
    let now: u64 = crate::time::now() as u64;

    events
        .iter()
        .filter(|e| now.saturating_sub(e.ts) < window_secs)
        .map(|e| e.wallet.clone())
        .collect::<HashSet<String>>()
        .into_iter()
        .collect()
}

/// Unique wallets in the last window_secs that were classified as whales (tier != None).
pub fn window_whales(events: &Vec<Event>, window_secs: u64) -> Vec<String> {
    let now: u64 = crate::time::now() as u64;

    events
        .iter()
        .filter(|e| now.saturating_sub(e.ts) < window_secs && e.tier != WhaleTier::None)
        .map(|e| e.wallet.clone())
        .collect::<HashSet<String>>()
        .into_iter()
        .collect()
}

/// Simple runner scoring based on growth in signers + tx between two windows.
pub fn runner_score(
    signers_now: usize,
    tx_now: usize,
    prev_signers: usize,
    prev_tx: usize,
) -> (i32, f64, f64) {
    let wallet_growth = if prev_signers > 0 {
        signers_now as f64 / prev_signers as f64
    } else {
        1.0
    };

    let tx_growth = if prev_tx > 0 {
        tx_now as f64 / prev_tx as f64
    } else {
        1.0
    };

    let score = 20.0 * (wallet_growth + tx_growth) / 2.0;
    (score as i32, wallet_growth, tx_growth)
}
