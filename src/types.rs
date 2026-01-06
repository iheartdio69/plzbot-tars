// types.rs
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
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallRecord {
    pub mint: String,
    pub call_ts: u64,
    pub score: i32,

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

// market/types.rs (stub for DexPair; expand as needed)
#[derive(Debug, Clone, Deserialize)]
pub struct DexPair {
    pub chain_id: String,
    pub base_token: TokenInfo,
    pub quote_token: TokenInfo,
    pub fdv: Option<f64>,
    pub liquidity: Option<Liquidity>,
    pub txns: Option<Txns>,
    // Add more fields as needed from Dexscreener response
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenInfo {
    pub address: String,
    pub symbol: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Liquidity {
    pub usd: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Txns {
    pub m5: Option<TxnCounts>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TxnCounts {
    pub buys: Option<u64>,
    pub sells: Option<u64>,
}

#[derive(Debug, Default)]
pub struct MarketTrend {
    pub last_fdv: Option<f64>,
    pub last_liq: Option<f64>,
    pub price_accel: f64, // Stub
    pub fdv_accel: f64,   // Stub
}