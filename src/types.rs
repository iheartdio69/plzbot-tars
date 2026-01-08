use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub enum WhaleTier {
    None,
    Beluga,
    Blue,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Event {
    pub wallet: String,
    pub ts: u64,
    pub sol: f64,
    pub tier: WhaleTier,
}

#[derive(Debug, Clone)]
pub struct CoinState {
    pub pair_address: Option<String>,
    pub first_seen: u64,
    pub events: Vec<Event>,

    // scoring / state machine
    pub score: i32,
    pub active: bool,
    pub low_score_streak: u8,

    // windows
    pub unique_signers_5m: usize,
    pub tx_5m: usize,
}

impl CoinState {
    pub fn new() -> Self {
        Self {
            pair_address: None,
            first_seen: crate::time::now(),
            events: vec![],
            score: 0,
            active: false,
            low_score_streak: 0,
            unique_signers_5m: 0,
            tx_5m: 0,
        }
    }
}

#[derive(Debug, Clone)]
pub struct CallRecord {
    pub mint: String,
    pub ts: u64,
    pub score: i32,
}

#[derive(Debug, Default, Clone, Serialize, Deserialize)]
pub struct WalletReputation {
    pub score: i32,
    pub hits: u64,
    pub rugs: u64,
}

pub type WalletRepMap = HashMap<String, WalletReputation>;
pub type RugSet = HashSet<String>;
