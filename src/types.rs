use serde::{Deserialize, Serialize};
use std::time::Instant;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WhaleTier {
    None,
    Beluga,
    Blue,
}

#[derive(Debug, Clone)]
pub struct Event {
    pub wallet: String,
    pub ts: u64,
    pub sol: f64,
    pub tier: WhaleTier,
}

#[derive(Debug, Clone)]
pub struct CoinState {
    pub mint: String,
    pub first_seen: Instant,
    pub last_snapshot: Instant,
    pub first_snapshot_done: bool,
    pub active: bool,
    pub low_score_streak: u32,
    pub prev_tx_window: usize,
    pub prev_signers_window: usize,
    pub events: Vec<Event>,
}

impl CoinState {
    pub fn new() -> Self {
        CoinState {
            mint: String::new(),
            first_seen: Instant::now(),
            last_snapshot: Instant::now(),
            first_snapshot_done: false,
            active: false,
            low_score_streak: 0,
            prev_tx_window: 0,
            prev_signers_window: 0,
            events: Vec::new(),
        }
    }

    pub fn new_with_mint(mint: String) -> Self {
        CoinState {
            mint,
            ..CoinState::new()
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallRecord {
    pub mint: String,
    pub call_ts: u64,
    pub score: i32,
    pub fdv_at_call: f64,  // FDV at the moment we called it — baseline for mult calc

    pub t5_ts: Option<u64>,
    pub wallets_t5: Option<usize>,
    pub tx_t5: Option<usize>,

    pub t15_ts: Option<u64>,
    pub wallets_t15: Option<usize>,
    pub tx_t15: Option<usize>,

    pub outcome: Option<String>,

    pub wallets_involved: Vec<String>,
    pub whales_involved: Vec<String>,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WalletStats {
    pub wins: u32,
    pub losses: u32,
    pub score: i32,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct WhalePerf {
    pub wins: u32,
    pub losses: u32,
    pub score: f64,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Usage {
    pub day: u64,
    pub requests: u64,
}
