use crate::config::Config;
use crate::market::cache::{market_trend, MarketCache};
use crate::rug_tracker::{record_loss, record_win, save_rug_tracker, WalletStrike};
use crate::time::now_ts;
use crate::types::CallRecord;
use colored::*;
use std::collections::HashMap;

pub fn resolve_calls(
    cfg: &Config,
    market: &MarketCache,
    calls: &mut Vec<CallRecord>,
    rug_tracker: &mut HashMap<String, WalletStrike>,
    _tg_token: &str,
    _tg_chat: &str,
) -> Vec<(String, String)> {
    let now = now_ts();
    let mut alerts: Vec<(String, String)> = Vec::new();
    let mut tracker_dirty = false;

    for call in calls.iter_mut() {
        if call.outcome.is_some() {
            continue;
        }

        let elapsed = now.saturating_sub(call.call_ts);
        if elapsed < 300 { continue; } // need at least 5 min

        let trend = market_trend(market, &call.mint, cfg);
        let current_fdv = match trend.last_fdv {
            Some(f) if f > 0.0 => f,
            _ => continue,
        };

        let liq = trend.last_liq.unwrap_or(0.0);
        let elapsed_mins = elapsed / 60;

        // Use locked call FDV as baseline
        let call_fdv = if call.fdv_at_call > 0.0 {
            call.fdv_at_call
        } else {
            current_fdv // fallback
        };

        // Update peak — track highest price seen after entry
        let current_mult = if call_fdv > 0.0 { current_fdv / call_fdv } else { 1.0 };
        if current_mult > call.peak_mult {
            call.peak_mult = current_mult;
            call.peak_fdv = current_fdv;
        }

        let peak_mult = call.peak_mult;
        let down_from_peak = if call.peak_fdv > 0.0 {
            (call.peak_fdv - current_fdv) / call.peak_fdv
        } else { 0.0 };

        // ── RESOLUTION LOGIC ──────────────────────────────────────────
        // WIN = peaked at 2x+ from entry (we could have taken profit)
        // LOSS = dropped 30%+ from entry price (never recovered)
        // Still riding = hasn't peaked or died yet

        let liq_dead = liq < 500.0 && elapsed > 600;
        let hard_sl = current_mult <= 0.10; // only exit if 90%+ gone = actual rug
        let stale = elapsed >= cfg.resolve_t15_secs && current_mult < 1.05; // 2hr, barely moved
        let slow_bleed = elapsed >= 7200 && current_mult < 0.5; // 4hr and down 50%
        let max_time = elapsed >= 86400; // 24hr max hold

        // ── TRAILING PROFIT LOCK ──────────────────────────────────────
        // Below 3x: hold through everything — normal meme volatility
        // Hit 3x+: protect profits if it reverses back toward 2.2x
        // Hit 5x+: floor rises to 3x
        // Hit 10x+: floor rises to 6x
        // This way we NEVER sell on the way up, only on meaningful reversals

        let trailing_exit = if peak_mult >= 10.0 && current_mult <= 6.0 {
            Some(format!("trail lock: peaked {:.1}x, fell to {:.1}x (floor 6x)", peak_mult, current_mult))
        } else if peak_mult >= 5.0 && current_mult <= 3.0 {
            Some(format!("trail lock: peaked {:.1}x, fell to {:.1}x (floor 3x)", peak_mult, current_mult))
        } else if peak_mult >= 3.0 && current_mult <= 2.2 {
            Some(format!("trail lock: peaked {:.1}x, fell to {:.1}x (floor 2.2x)", peak_mult, current_mult))
        } else {
            None
        };

        // WIN conditions
        let is_win = peak_mult >= 2.2;
        let is_mid = peak_mult >= 1.3 && peak_mult < 2.2;

        // Keep riding if nothing triggered
        let should_exit = liq_dead || hard_sl || stale || slow_bleed || max_time || trailing_exit.is_some();

        if !should_exit {
            if elapsed_mins % 30 == 0 && elapsed_mins > 0 {
                println!("{}", format!(
                    "⏳ RIDING {} | now {:.2}x | peak {:.2}x | {}min in",
                    &call.mint[..12], current_mult, peak_mult, elapsed_mins
                ).bright_black());
            }
            continue;
        }

        // Resolve
        call.t15_ts = Some(now);
        tracker_dirty = true;

        let trail_reason = trailing_exit.unwrap_or_default();
        let (outcome, reason) = if is_win {
            let r = if !trail_reason.is_empty() { trail_reason.clone() } else { format!("peaked {:.2}x", peak_mult) };
            ("WIN", r)
        } else if is_mid {
            ("MID", format!("peaked {:.2}x", peak_mult))
        } else if liq_dead {
            ("LOSS", "liq pulled".to_string())
        } else if hard_sl {
            ("LOSS", "rug — 90%+ gone".to_string())
        } else {
            ("LOSS", format!("peak only {:.2}x", peak_mult))
        };

        call.outcome = Some(outcome.to_string());

        match outcome {
            "WIN" => {
                println!("{}", format!(
                    "✅ WIN → {} | peak {:.2}x | now {:.2}x | {}min",
                    &call.mint[..12], peak_mult, current_mult, elapsed_mins
                ).bold().bright_green());
                alerts.push((call.mint.clone(), format!(
                    "WIN|{:.2}|{}", peak_mult, reason
                )));
                record_win(rug_tracker, &call.wallets_involved);
            }
            "MID" => {
                println!("{}", format!(
                    "➖ MID → {} | peak {:.2}x | {}min",
                    &call.mint[..12], peak_mult, elapsed_mins
                ).bright_black());
            }
            _ => {
                println!("{}", format!(
                    "❌ LOSS → {} | peak {:.2}x | {} | {}min",
                    &call.mint[..12], peak_mult, reason, elapsed_mins
                ).bold().red());
                alerts.push((call.mint.clone(), format!(
                    "LOSS|{:.2}|{}", current_mult, reason
                )));
                record_loss(rug_tracker, &call.wallets_involved);
            }
        }
    }

    if tracker_dirty {
        save_rug_tracker(rug_tracker);
    }

    alerts
}
