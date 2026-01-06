// scoring/window.rs
use crate::config::Config;
use crate::types::{Event, WhaleTier};

#[derive(Debug, Default)]
pub struct Whales {
    pub beluga_count: usize,
    pub blue_count: usize,
}

pub fn prune_window(events: &mut Vec<Event>, keep_secs: u64) {
    let now = now_ts();
    events.retain(|e| now - e.ts < keep_secs);
}

pub fn window_stats_for(events: &Vec<Event>, window_secs: u64) -> (usize, usize, Whales) {
    let now = now_ts();
    let recent: Vec<&Event> = events.iter().filter(|e| now - e.ts < window_secs).collect();
    let tx_now = recent.len();
    let signers_now = recent.iter().map(|e| &e.wallet).collect::<HashSet<_>>().len();
    let beluga = recent.iter().filter(|e| e.tier == WhaleTier::Beluga).count();
    let blue = recent.iter().filter(|e| e.tier == WhaleTier::Blue).count();
    (tx_now, signers_now, Whales { beluga_count: beluga, blue_count: blue })
}

pub fn window_wallets(events: &Vec<Event>, window_secs: u64) -> Vec<String> {
    let now = now_ts();
    events.iter().filter(|e| now - e.ts < window_secs).map(|e| e.wallet.clone()).collect::<HashSet<_>>().into_iter().collect()
}

pub fn window_whales(events: &Vec<Event>, window_secs: u64) -> Vec<String> {
    let now = now_ts();
    events.iter().filter(|e| now - e.ts < window_secs && e.tier != WhaleTier::None).map(|e| e.wallet.clone()).collect::<HashSet<_>>().into_iter().collect()
}

pub fn runner_score(signers_now: usize, tx_now: usize, prev_signers: usize, prev_tx: usize) -> (i32, f64, f64) {
    let wallet_growth = if prev_signers > 0 { signers_now as f64 / prev_signers as f64 } else { 1.0 };
    let tx_growth = if prev_tx > 0 { tx_now as f64 / prev_tx as f64 } else { 1.0 };
    let score = 20.0 * (wallet_growth + tx_growth) / 2.0;
    (score as i32, wallet_growth, tx_growth)
}