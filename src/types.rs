use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum WhaleTier {
    None,
    Blue,
    Beluga,
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
    // -------------------------
    // identity / lifecycle
    // -------------------------
    pub last_called_ts: u64,
    pub pair_address: Option<String>,
    pub first_seen: u64,

    // last time we saw *any meaningful activity* (events, tx movement, etc.)
    pub last_activity_ts: u64,

    // -------------------------
    // event buffer
    // -------------------------
    pub events: Vec<Event>,

    // -------------------------
    // scoring / state machine
    // -------------------------
    pub score: i32,
    pub active: bool,
    pub active_since: u64,

    // how many consecutive ticks we’ve been “too low score”
    pub low_score_streak: u8,

    // last time we wrote a mint_snapshot row for this mint (unix seconds)
    // (in-memory heartbeat to avoid extra DB lookups)
    pub last_snapshot_ts: i64,

    // -------------------------
    // queue tracking
    // -------------------------
    pub queued_since: u64,

    // -------------------------
    // rolling windows (engine-maintained)
    // -------------------------
    pub unique_signers_5m: usize,
    pub tx_5m: usize,

    // -------------------------
    // gates / cached decision helpers
    // -------------------------
    pub skip_call_for_conc: bool,
    pub is_recovery: bool,
    pub is_volume_spike: bool,
    pub whale_entry: bool,
    pub whale_entry_score: i64,
    pub prev_tx_5m: usize,
    pub prev_tx_ts: u64,
    pub wallet_delta: i32,

    // -------------------------
    // pump.fun launch data (from PumpPortal, free at discovery)
    // -------------------------
    pub launch_sol: Option<f64>,       // vSolInBondingCurve at first seen
    pub creator_wallet: Option<String>, // deployer wallet — checked vs rug DB
    pub creator_is_rug: bool,           // true = creator flagged in wallets table
    pub social_score: i32,              // points for twitter/telegram/website at launch
    pub has_socials: bool,              // has at least one social link

    // -------------------------
    // enrichment data (async background fetch)
    // -------------------------
    pub holder_count: Option<u64>,     // real holder count from Helius
    pub top_holder_pct: Option<f64>,   // top 1 holder % — rug signal if > 20%
    pub is_graduated: bool,            // tradeable on Jupiter = graduated pump.fun
    pub dex_has_socials: bool,         // DexScreener shows twitter/website
    pub dex_boost_active: u64,         // active DexScreener boosts (marketing signal)
    pub enrichment_done: bool,         // have we fetched enrichment data yet?

    // -------------------------
    // sol flow (from events)
    // -------------------------
    pub sol_flow_5m: f64,              // cached SOL flow last 5m from event buffer

    // -------------------------
    // active clog control (demotion logic)
    // -------------------------
    pub demote_streak: u32,
    pub last_demote_ts: i64,
}

impl CoinState {
    pub fn new() -> Self {
        let now_u64 = crate::time::now();
        let now_i64 = now_u64 as i64;

        Self {
            last_called_ts: 0,
            pair_address: None,
            first_seen: now_u64,
            last_activity_ts: now_u64,

            events: Vec::new(),

            score: 0,
            active: false,
            active_since: 0,
            low_score_streak: 0,
            last_snapshot_ts: 0,

            queued_since: 0,

            unique_signers_5m: 0,
            tx_5m: 0,

            skip_call_for_conc: false,
            is_recovery: false,
            is_volume_spike: false,
            whale_entry: false,
            whale_entry_score: 0,
            prev_tx_5m: 0,
            prev_tx_ts: 0,
            wallet_delta: 0,

            launch_sol: None,
            creator_wallet: None,
            creator_is_rug: false,
            social_score: 0,
            has_socials: false,

            holder_count: None,
            top_holder_pct: None,
            is_graduated: false,
            dex_has_socials: false,
            dex_boost_active: 0,
            enrichment_done: false,

            sol_flow_5m: 0.0,

            demote_streak: 0,
            last_demote_ts: 0,
        }
    }

    /// Optional helper: mark that we wrote a snapshot “now”.
    pub fn touch_snapshot(&mut self) {
        self.last_snapshot_ts = crate::time::now() as i64;
    }

    /// Optional helper: update activity “now”.
    pub fn touch_activity(&mut self) {
        self.last_activity_ts = crate::time::now();
    }

    /// Optional helper: mark queued “now”.
    pub fn mark_queued(&mut self) {
        self.queued_since = crate::time::now();
    }

    /// Optional helper: mark active “now”.
    pub fn mark_active(&mut self) {
        let now = crate::time::now();
        self.active = true;
        self.active_since = now;
        self.queued_since = 0;
    }

    /// Optional helper: mark inactive.
    pub fn mark_inactive(&mut self) {
        self.active = false;
        self.active_since = 0;
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
