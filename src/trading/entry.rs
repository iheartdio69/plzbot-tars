/// Entry timing logic — dip sniper
///
/// When a CALL is made, don't buy the green candle.
/// Wait for the first pullback then enter on recovery.
/// Only bypass this for rockets moving so fast it's dumb to wait.

use crate::market::cache::MarketCache;
use crate::config::Config;
use std::time::{Duration, Instant};
use tokio::time::sleep;

/// Result of the dip-wait process
#[derive(Debug)]
pub enum EntryDecision {
    /// Enter now at this FDV
    Enter { fdv: f64, reason: String },
    /// Skip — coin died before we could enter
    Skip { reason: String },
}

/// How far price must dip from call price before we consider it a "dip" (%)
const DIP_THRESHOLD_PCT: f64 = 2.0;

/// If price drops this far from call price, coin is dying — bail
const BAIL_THRESHOLD_PCT: f64 = 15.0;

/// FDV velocity that triggers immediate bypass (% per min)
/// For rockets so hot that waiting is stupid
const ROCKET_VELOCITY_BYPASS: f64 = 25.0;

/// How long to wait for a dip before buying anyway (seconds)
const DIP_WAIT_TIMEOUT_SECS: u64 = 60;

/// How often to sample price while waiting (seconds)
const SAMPLE_INTERVAL_SECS: u64 = 2;

pub async fn wait_for_entry(
    mint: &str,
    call_fdv: f64,
    call_velocity: f64,
    market: &MarketCache,
    cfg: &Config,
) -> EntryDecision {
    // ── ROCKET BYPASS ─────────────────────────────────────────────────
    // Moving so fast that waiting for a dip means missing the trade entirely
    if call_velocity >= ROCKET_VELOCITY_BYPASS {
        println!(
            "  🚀 ROCKET BYPASS — vel {:.1}%/min ≥ {:.0}% threshold, entering immediately",
            call_velocity, ROCKET_VELOCITY_BYPASS
        );
        return EntryDecision::Enter {
            fdv: call_fdv,
            reason: format!("rocket bypass ({:.1}%/min)", call_velocity),
        };
    }

    println!(
        "  ⏳ DIP WAIT — call FDV ${:.0} | vel {:.1}%/min | waiting for pullback...",
        call_fdv, call_velocity
    );

    let deadline = Instant::now() + Duration::from_secs(DIP_WAIT_TIMEOUT_SECS);
    let mut lowest_fdv = call_fdv;
    let mut dip_seen = false;

    loop {
        sleep(Duration::from_secs(SAMPLE_INTERVAL_SECS)).await;

        // Get latest price snapshot
        let current_fdv = match get_latest_fdv(market, mint) {
            Some(f) => f,
            None => {
                if Instant::now() >= deadline {
                    return EntryDecision::Skip {
                        reason: "no market data and timeout reached".to_string(),
                    };
                }
                continue;
            }
        };

        let change_from_call = (current_fdv - call_fdv) / call_fdv * 100.0;
        let change_from_low = (current_fdv - lowest_fdv) / lowest_fdv * 100.0;

        // Track lowest seen
        if current_fdv < lowest_fdv {
            lowest_fdv = current_fdv;
        }

        let dip_pct = (call_fdv - lowest_fdv) / call_fdv * 100.0;

        // ── BAIL: coin is dying ────────────────────────────────────────
        if change_from_call <= -(BAIL_THRESHOLD_PCT) {
            return EntryDecision::Skip {
                reason: format!(
                    "coin dropped {:.1}% from call — bailing",
                    change_from_call.abs()
                ),
            };
        }

        // ── DIP DETECTED ──────────────────────────────────────────────
        if dip_pct >= DIP_THRESHOLD_PCT {
            dip_seen = true;
        }

        // ── ENTER ON RECOVERY ─────────────────────────────────────────
        // Price dipped enough AND is now recovering (moving back up from low)
        if dip_seen && change_from_low >= 1.0 {
            return EntryDecision::Enter {
                fdv: current_fdv,
                reason: format!(
                    "dip {:.1}% → recovery +{:.1}% (entry at ${:.0})",
                    dip_pct, change_from_low, current_fdv
                ),
            };
        }

        // ── TIMEOUT: buy anyway rather than miss the whole move ────────
        if Instant::now() >= deadline {
            println!(
                "  ⌛ Dip timeout — no pullback in {}s, entering at market (FDV ${:.0})",
                DIP_WAIT_TIMEOUT_SECS, current_fdv
            );
            return EntryDecision::Enter {
                fdv: current_fdv,
                reason: format!(
                    "timeout {}s — no dip seen, entering at market",
                    DIP_WAIT_TIMEOUT_SECS
                ),
            };
        }

        println!(
            "  👀 {} | FDV ${:.0} | call Δ {:+.1}% | low Δ {:.1}% | dip_seen={}",
            &mint[..8], current_fdv, change_from_call, dip_pct, dip_seen
        );
    }
}

fn get_latest_fdv(market: &MarketCache, mint: &str) -> Option<f64> {
    market.map.get(mint)?.last()?.fdv
}

/// Store entry timing metadata alongside a call for logging/analysis
#[derive(Debug, Clone)]
pub struct EntryMeta {
    pub call_fdv: f64,
    pub actual_entry_fdv: f64,
    pub entry_reason: String,
    pub dip_saved_pct: f64, // how much better our entry was vs buying the green candle
}

impl EntryMeta {
    pub fn new(call_fdv: f64, actual_entry_fdv: f64, reason: String) -> Self {
        let dip_saved_pct = (call_fdv - actual_entry_fdv) / call_fdv * 100.0;
        Self {
            call_fdv,
            actual_entry_fdv,
            entry_reason: reason,
            dip_saved_pct,
        }
    }
}
