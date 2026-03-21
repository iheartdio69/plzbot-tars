use crate::config::Config;
use crate::market::cache::{market_trend, MarketCache};
use crate::time::now_ts;
use crate::types::CallRecord;
use colored::*;

// Resolve calls based on FDV change from call time
// WIN  = FDV 2x+ from call price
// MID  = FDV 1.3x–2x
// LOSS = FDV < 1.3x or dumped below call price

pub fn resolve_calls(
    cfg: &Config,
    market: &MarketCache,
    calls: &mut Vec<CallRecord>,
    tg_token: &str,
    tg_chat: &str,
) -> Vec<(String, String)> {
    let now = now_ts();
    let mut alerts: Vec<(String, String)> = Vec::new();

    for call in calls.iter_mut() {
        if call.outcome.is_some() {
            continue;
        }

        let elapsed = now.saturating_sub(call.call_ts);

        // Need at least T+5 to resolve
        if elapsed < cfg.resolve_t5_secs {
            continue;
        }

        // Get current FDV from market cache
        let trend = market_trend(market, &call.mint, cfg);
        let current_fdv = match trend.last_fdv {
            Some(f) if f > 0.0 => f,
            _ => continue, // no data yet
        };

        // T+5 snapshot
        if call.t5_ts.is_none() && elapsed >= cfg.resolve_t5_secs {
            call.t5_ts = Some(now);
            // Store FDV at T5 in wallets_t5 as a proxy (reusing field)
            // We'll store the FDV*100 as an integer for now
            call.wallets_t5 = Some((current_fdv / 100.0) as usize);
        }

        // Final resolution at T+15
        if elapsed >= cfg.resolve_t15_secs {
            call.t15_ts = Some(now);

            // Reconstruct call FDV from score (rough) — or use stored T5
            // Better: compare to first known FDV from market cache oldest snapshot
            let call_fdv = market.map.get(&call.mint)
                .and_then(|snaps| snaps.first())
                .and_then(|s| s.fdv)
                .unwrap_or(current_fdv);

            let mult = if call_fdv > 0.0 { current_fdv / call_fdv } else { 1.0 };

            let outcome = if mult >= 2.0 {
                "WIN"
            } else if mult >= 1.3 {
                "MID"
            } else {
                "LOSS"
            };

            call.outcome = Some(outcome.to_string());

            match outcome {
                "WIN" => {
                    let msg = format!(
                        "✅ WIN → {} | {:.2}x | FDV ${:.0}",
                        &call.mint[..12], mult, current_fdv
                    );
                    println!("{}", msg.bold().bright_green());
                    alerts.push((call.mint.clone(), format!(
                        "✅ <b>WIN</b> {:.2}x\n{}\nFDV now: ${:.0}",
                        mult, call.mint, current_fdv
                    )));
                }
                "MID" => {
                    println!("{}", format!(
                        "➖ MID → {} | {:.2}x", &call.mint[..12], mult
                    ).bright_black());
                }
                _ => {
                    let msg = format!(
                        "❌ LOSS → {} | {:.2}x | FDV ${:.0}",
                        &call.mint[..12], mult, current_fdv
                    );
                    println!("{}", msg.bold().red());
                    alerts.push((call.mint.clone(), format!(
                        "❌ <b>LOSS</b> {:.2}x\n{}\nFDV now: ${:.0}",
                        mult, call.mint, current_fdv
                    )));
                }
            }
        }
    }

    alerts
}
