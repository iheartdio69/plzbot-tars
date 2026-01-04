use serde::{Deserialize, Serialize};
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

pub type MarketCache = HashMap<String, VecDeque<MarketSnap>>;

#[derive(Debug, Clone)]
pub struct MarketSnap {
    pub ts: u64,
    pub price_usd: Option<f64>,
    pub fdv: Option<f64>,
    pub liquidity_usd: Option<f64>,
    pub vol_h24: Option<f64>,
}

#[derive(Default, Debug, Clone)]
pub struct MarketTrend {
    pub price_up: bool,
    pub fdv_ok: bool,
    pub liq_ok: bool,
    pub last_price: Option<f64>,
    pub last_fdv: Option<f64>,
    pub last_liq: Option<f64>,
}

// ===== Helius types =====
#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct HeliusTx {
    pub signature: String,
    pub timestamp: u64,

    #[serde(default)]
    pub fee_payer: Option<String>,

    #[serde(default)]
    pub token_transfers: Vec<TokenTransfer>,

    #[serde(default)]
    pub native_transfers: Vec<NativeTransfer>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct TokenTransfer {
    #[serde(default)]
    pub mint: Option<String>,
}

#[derive(Debug, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct NativeTransfer {
    #[serde(default)]
    pub from_user_account: Option<String>,
    #[serde(default)]
    pub amount: u64,
}

// ===== Data =====
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

#[derive(Debug)]
pub struct CoinState {
    pub first_seen: Instant,
    pub last_update: Instant,
    pub events: Vec<Event>,
    pub active: bool,

    pub last_snapshot: Instant,
    pub prev_tx_window: usize,
    pub prev_signers_window: usize,
    pub low_score_streak: u8,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct WalletStats {
    pub seen: u32,
    pub wins: u32,
    pub losses: u32,
    pub score: i32,
    pub last_seen_ts: u64,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct WhalePerf {
    pub seen: u32,
    pub beluga_txs: u32,
    pub blue_txs: u32,
    pub beluga_sol: f64,
    pub blue_sol: f64,
    pub wins: u32,
    pub losses: u32,
    pub score: f64,
    pub last_seen_ts: u64,
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

    pub outcome: Option<String>, // "WIN" | "MID" | "LOSS"
    pub wallets_involved: Vec<String>,
    pub whales_involved: Vec<String>,
}

#[derive(Serialize, Deserialize, Default, Debug, Clone)]
pub struct Usage {
    pub day: u64,
    pub requests: u64,
}

#[derive(Debug, Default, Clone)]
pub struct WhaleWindow {
    pub beluga_count: usize,
    pub blue_count: usize,
}