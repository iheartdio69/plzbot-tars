// missed_calls.rs
// Tracks coins that pump to $150k+ that we never called.
// Every tick, check uncalled coins — if any hit the threshold, log them
// as missed opportunities so we can analyze what we should have caught.

use crate::market::cache::{market_trend, MarketCache};
use crate::config::Config;
use crate::types::CoinState;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};

const MISSED_PATH: &str = "data/missed_calls.json";
const PUMP_THRESHOLD: f64 = 150_000.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MissedCall {
    pub mint: String,
    pub peak_fdv: f64,
    pub first_seen_fdv: f64,
    pub ts: u64,
    pub snapshots_before_pump: usize,
    pub max_buys_5m_seen: u64,
    pub max_velocity_seen: f64,
    pub max_bsr_seen: f64,
    pub was_in_fdv_band: bool,
    pub skip_reason: String,
}

#[derive(Debug, Default)]
pub struct MissedCallTracker {
    pub already_logged: HashSet<String>,
    // Per-coin tracking: (first_fdv, max_buys, max_vel, max_bsr, snapshots, in_band)
    pub history: HashMap<String, CoinHistory>,
}

#[derive(Debug, Clone, Default)]
pub struct CoinHistory {
    pub first_fdv: f64,
    pub max_buys_5m: u64,
    pub max_velocity: f64,
    pub max_bsr: f64,
    pub snapshots: usize,
    pub ever_in_fdv_band: bool,
}

impl MissedCallTracker {
    pub fn load() -> Self {
        let mut tracker = Self::default();
        if let Ok(s) = std::fs::read_to_string(MISSED_PATH) {
            if let Ok(missed) = serde_json::from_str::<Vec<MissedCall>>(&s) {
                for m in missed {
                    tracker.already_logged.insert(m.mint);
                }
            }
        }
        tracker
    }

    pub fn update(
        &mut self,
        mint: &str,
        fdv: f64,
        buys_5m: u64,
        velocity: f64,
        bsr: f64,
        cfg: &Config,
        was_called: bool,
    ) {
        if self.already_logged.contains(mint) {
            return;
        }

        let h = self.history.entry(mint.to_string()).or_default();
        if h.first_fdv == 0.0 { h.first_fdv = fdv; }
        h.snapshots += 1;
        if buys_5m > h.max_buys_5m { h.max_buys_5m = buys_5m; }
        if velocity > h.max_velocity { h.max_velocity = velocity; }
        if bsr > h.max_bsr { h.max_bsr = bsr; }
        if fdv >= cfg.min_call_fdv_usd && fdv <= cfg.max_call_fdv_usd {
            h.ever_in_fdv_band = true;
        }

        // If it hit the pump threshold and we never called it — log as missed
        let should_log = fdv >= PUMP_THRESHOLD && !was_called;
        let h_clone = h.clone();
        if should_log {
            self.log_missed(mint, fdv, h_clone, cfg);
        }
    }

    fn log_missed(&mut self, mint: &str, peak_fdv: f64, h: CoinHistory, cfg: &Config) {
        self.already_logged.insert(mint.to_string());

        // Figure out why we missed it
        let skip_reason = if !h.ever_in_fdv_band {
            format!("FDV never in call band (${:.0}–${:.0})", cfg.min_call_fdv_usd, cfg.max_call_fdv_usd)
        } else if h.max_buys_5m < cfg.min_buys_5m as u64 {
            format!("Buys too low (max seen: {}, needed: {})", h.max_buys_5m, cfg.min_buys_5m)
        } else if h.max_velocity < 0.0 {
            "FDV velocity was negative when scanned".to_string()
        } else if h.max_velocity < cfg.fdv_velocity_threshold {
            format!("Velocity too low (max: {:.1}%/min, threshold: {:.1})", h.max_velocity, cfg.fdv_velocity_threshold)
        } else {
            "Score didn't clear threshold".to_string()
        };

        let missed = MissedCall {
            mint: mint.to_string(),
            peak_fdv,
            first_seen_fdv: h.first_fdv,
            ts: crate::time::now_ts(),
            snapshots_before_pump: h.snapshots,
            max_buys_5m_seen: h.max_buys_5m,
            max_velocity_seen: h.max_velocity,
            max_bsr_seen: h.max_bsr,
            was_in_fdv_band: h.ever_in_fdv_band,
            skip_reason: skip_reason.clone(),
        };

        println!(
            "🔍 MISSED PUMP → {} | Peak FDV ${:.0} | Reason: {}",
            &mint[..12.min(mint.len())],
            peak_fdv,
            skip_reason
        );

        // Save
        let mut existing: Vec<MissedCall> = std::fs::read_to_string(MISSED_PATH)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default();
        existing.push(missed);
        if let Ok(s) = serde_json::to_string_pretty(&existing) {
            let _ = std::fs::write(MISSED_PATH, s);
        }
    }
}
