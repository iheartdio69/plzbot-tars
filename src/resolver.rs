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
        let elapsed_mins = elapsed / 60;

        // Need at least 5 minutes before any resolution
        if elapsed < 300 {
            continue;
        }

        let trend = market_trend(market, &call.mint, cfg);
        let current_fdv = match trend.last_fdv {
            Some(f) if f > 0.0 => f,
            _ => continue,
        };

        // Get baseline FDV from first snapshot
        let call_fdv = market.map.get(&call.mint)
            .and_then(|snaps| snaps.first())
            .and_then(|s| s.fdv)
            .unwrap_or(current_fdv);

        let mult = if call_fdv > 0.0 { current_fdv / call_fdv } else { 1.0 };

        // T+30 snapshot
        if call.t5_ts.is_none() && elapsed >= cfg.resolve_t5_secs {
            call.t5_ts = Some(now);
            call.wallets_t5 = Some((current_fdv / 100.0) as usize);
        }

        // ── DYNAMIC RESOLUTION ────────────────────────────────────────
        // Don't wait for a fixed time — resolve on conditions

        let liq = trend.last_liq.unwrap_or(0.0);
        let bsr = trend.buy_sell_ratio;
        let vel = trend.fdv_velocity_pct;
        let price_change_1h = trend.price_change_1h;

        // INSTANT WIN — 2x+ at any point
        let is_win = mult >= 2.0;

        // STRONG WIN — 2x+ after 30min
        let is_strong = mult >= 2.0 && elapsed >= cfg.resolve_t5_secs;

        // Runner detection removed — let winners ride
        let is_runner_win = false; // don't exit early, ride to max

        // DEATH SIGNALS — resolve as loss
        let liq_dead = liq < 1_000.0 && elapsed >= 600; // liquidity pulled
        let price_tanking = mult <= 0.70; // -30% hard stop
        let stale_and_dead = elapsed >= cfg.resolve_t15_secs && mult < 1.1 && bsr < 0.8;
        let slow_bleed = elapsed >= 3600 && mult < 0.85; // been an hour, down 15%+

        // GRINDER STILL ALIVE — don't resolve yet, keep watching
        let grinder_alive = mult >= 0.90 && bsr >= 1.0 && liq >= 3_000.0 && elapsed < 43200; // up to 4hrs

        let should_resolve = is_win || is_strong || is_runner_win
            || liq_dead || price_tanking || stale_and_dead || slow_bleed;

        // If coin is still healthy, keep riding
        if !should_resolve && grinder_alive {
            if elapsed_mins % 30 == 0 && elapsed_mins > 0 {
                println!(
                    "{}",
                    format!(
                        "⏳ RIDING {} | {:.2}x | vel {:.1}%/min | BSR {:.1}x | {}min in",
                        &call.mint[..12], mult, vel, bsr, elapsed_mins
                    ).bright_black()
                );
            }
            continue;
        }

        if !should_resolve {
            continue;
        }

        // Resolve
        call.t15_ts = Some(now);
        call.wallets_t15 = Some((current_fdv / 100.0) as usize);
        tracker_dirty = true;

        let outcome = if is_win || is_strong || is_runner_win {
            "WIN"
        } else if mult >= 1.3 {
            "MID"
        } else {
            "LOSS"
        };

        call.outcome = Some(outcome.to_string());

        let reason = if liq_dead { "liq pulled" }
            else if price_tanking { "-30% SL" }
            else if stale_and_dead { "stale+dead" }
            else if slow_bleed { "slow bleed" }
            else if is_runner_win { "runner!" }
            else { "target hit" };

        match outcome {
            "WIN" => {
                println!("{}", format!(
                    "✅ WIN → {} | {:.2}x | {} | {}min",
                    &call.mint[..12], mult, reason, elapsed_mins
                ).bold().bright_green());
                alerts.push((call.mint.clone(), format!(
                    "✅ <b>WIN {:.2}x</b> ({})\n{}\n<a href=\"https://dexscreener.com/solana/{}\">Chart</a>",
                    mult, reason, call.mint, call.mint
                )));
                record_win(rug_tracker, &call.wallets_involved);
            }
            "MID" => {
                println!("{}", format!(
                    "➖ MID → {} | {:.2}x | {}min", &call.mint[..12], mult, elapsed_mins
                ).bright_black());
            }
            _ => {
                println!("{}", format!(
                    "❌ LOSS → {} | {:.2}x | {} | {}min",
                    &call.mint[..12], mult, reason, elapsed_mins
                ).bold().red());
                alerts.push((call.mint.clone(), format!(
                    "❌ <b>LOSS {:.2}x</b> ({})\n{}",
                    mult, reason, call.mint
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
