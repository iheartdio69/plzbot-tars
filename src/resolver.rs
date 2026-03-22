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

        if elapsed < cfg.resolve_t5_secs {
            continue;
        }

        let trend = market_trend(market, &call.mint, cfg);
        let current_fdv = match trend.last_fdv {
            Some(f) if f > 0.0 => f,
            _ => continue,
        };

        if call.t5_ts.is_none() && elapsed >= cfg.resolve_t5_secs {
            call.t5_ts = Some(now);
            call.wallets_t5 = Some((current_fdv / 100.0) as usize);
        }

        if elapsed >= cfg.resolve_t15_secs {
            call.t15_ts = Some(now);

            let call_fdv = market.map.get(&call.mint)
                .and_then(|snaps| snaps.first())
                .and_then(|s| s.fdv)
                .unwrap_or(current_fdv);

            let mult = if call_fdv > 0.0 { current_fdv / call_fdv } else { 1.0 };

            let outcome = if mult >= 2.0 {
                "WIN"
            } else if mult >= 1.3 {
                "MID"
            } else if mult <= 0.70 {
                "LOSS" // -30% SL triggered
            } else {
                "LOSS" // general loss
            };

            call.outcome = Some(outcome.to_string());
            tracker_dirty = true;

            match outcome {
                "WIN" => {
                    println!("{}", format!(
                        "✅ WIN → {} | {:.2}x | FDV ${:.0}", &call.mint[..12], mult, current_fdv
                    ).bold().bright_green());
                    alerts.push((call.mint.clone(), format!(
                        "✅ <b>WIN</b> {:.2}x\n{}\nFDV now: ${:.0}", mult, call.mint, current_fdv
                    )));
                    record_win(rug_tracker, &call.wallets_involved);
                }
                "MID" => {
                    println!("{}", format!(
                        "➖ MID → {} | {:.2}x", &call.mint[..12], mult
                    ).bright_black());
                }
                _ => {
                    println!("{}", format!(
                        "❌ LOSS → {} | {:.2}x | FDV ${:.0}", &call.mint[..12], mult, current_fdv
                    ).bold().red());
                    alerts.push((call.mint.clone(), format!(
                        "❌ <b>LOSS</b> {:.2}x\n{}\nFDV now: ${:.0}", mult, call.mint, current_fdv
                    )));
                    record_loss(rug_tracker, &call.wallets_involved);
                }
            }
        }
    }

    if tracker_dirty {
        save_rug_tracker(rug_tracker);
    }

    alerts
}
