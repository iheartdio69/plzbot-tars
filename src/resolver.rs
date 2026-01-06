use crate::config::Config;
use crate::scoring::{window_stats_for, WhaleWindow};
use crate::time::now;
use crate::types::{CallRecord, CoinState, Event, WalletStats, WhalePerf, WhaleTier};
use colored::*;
use std::collections::{HashMap, HashSet};

fn stats_since(events: &[Event], since_ts: u64) -> (usize, usize, WhaleWindow) {
    let mut uniq = HashSet::<&str>::new();
    let mut beluga = HashSet::<&str>::new();
    let mut blue = HashSet::<&str>::new();
    let mut tx = 0usize;

    for e in events.iter() {
        if e.ts < since_ts {
            continue;
        }
        tx += 1;
        uniq.insert(&e.wallet);

        match e.tier {
            WhaleTier::Blue => {
                blue.insert(&e.wallet);
            }
            WhaleTier::Beluga => {
                beluga.insert(&e.wallet);
            }
            WhaleTier::None => {}
        }
    }

    (
        tx,
        uniq.len(),
        WhaleWindow {
            beluga_count: beluga.len(),
            blue_count: blue.len(),
        },
    )
}

pub fn resolver_tick(
    cfg: &Config,
    coins: &HashMap<String, CoinState>,
    calls: &mut Vec<CallRecord>,
    wallets: &mut HashMap<String, WalletStats>,
    whales: &mut HashMap<String, WhalePerf>,
) {
    let now_ts = now();

    for call in calls.iter_mut() {
        if call.outcome.is_some() {
            continue;
        }

        let elapsed = now_ts.saturating_sub(call.call_ts);

        // ---------- T+5 snapshot ----------
        if call.t5_ts.is_none() && elapsed >= cfg.resolve_t5_secs {
            if let Some(c) = coins.get(&call.mint) {
                // Use the SAME window logic used elsewhere (last WINDOW_SECS)
                let (tx_now, signers_now, _ww) = window_stats_for(&c.events, cfg.window_secs);

                call.t5_ts = Some(now_ts);
                call.wallets_t5 = Some(signers_now);
                call.tx_t5 = Some(tx_now);
            } else {
                // coin not found; still mark snapshot to avoid repeated attempts
                call.t5_ts = Some(now_ts);
                call.wallets_t5 = Some(0);
                call.tx_t5 = Some(0);
            }
        }

        // ---------- T+15 final resolution ----------
        if elapsed >= cfg.resolve_t15_secs {
            if let Some(c) = coins.get(&call.mint) {
                // For final stats, count everything since call time (not rolling window)
                let (tx_now, signers_now, _ww) = stats_since(&c.events, call.call_ts);

                call.t15_ts = Some(now_ts);
                call.wallets_t15 = Some(signers_now);
                call.tx_t15 = Some(tx_now);

                // baseline: t5 snapshot (avoid divide-by-zero)
                let w5 = call.wallets_t5.unwrap_or(0).max(1);
                let t5 = call.tx_t5.unwrap_or(0).max(1);

                let w_mult = (signers_now as f64) / (w5 as f64);
                let t_mult = (tx_now as f64) / (t5 as f64);

                let outcome = if w_mult >= cfg.win_wallet_mult || t_mult >= cfg.win_tx_mult {
                    "WIN"
                } else if w_mult >= cfg.mid_wallet_mult || t_mult >= cfg.mid_tx_mult {
                    "MID"
                } else {
                    "LOSS"
                };

                call.outcome = Some(outcome.to_string());

                match outcome {
                    "WIN" => {
                        println!(
                            "{}",
                            format!(
                                "✅ RESOLVED WIN: {}  (w {}→{} {:.2}x | tx {}→{} {:.2}x)",
                                call.mint, w5, signers_now, w_mult, t5, tx_now, t_mult
                            )
                            .bold()
                            .bright_green()
                        );
                    }
                    "MID" => {
                        println!(
                            "{}",
                            format!(
                                "➖ RESOLVED MID: {}  (w {}→{} {:.2}x | tx {}→{} {:.2}x)",
                                call.mint, w5, signers_now, w_mult, t5, tx_now, t_mult
                            )
                            .bright_black()
                        );
                    }
                    _ => {
                        println!(
                            "{}",
                            format!(
                                "❌ RESOLVED LOSS: {}  (w {}→{} {:.2}x | tx {}→{} {:.2}x)",
                                call.mint, w5, signers_now, w_mult, t5, tx_now, t_mult
                            )
                            .bold()
                            .red()
                        );
                    }
                }

                // Only adjust wallet/whale performance on WIN/LOSS (keep MID neutral)
                if outcome == "WIN" || outcome == "LOSS" {
                    for w in call.wallets_involved.iter() {
                        let ws = wallets.entry(w.clone()).or_default();
                        if outcome == "WIN" {
                            ws.wins = ws.wins.saturating_add(1);
                            ws.score = ws.score.saturating_add(6);
                        } else {
                            ws.losses = ws.losses.saturating_add(1);
                            ws.score = ws.score.saturating_sub(2);
                        }
                    }

                    for w in call.whales_involved.iter() {
                        let wp = whales.entry(w.clone()).or_default();
                        if outcome == "WIN" {
                            wp.wins = wp.wins.saturating_add(1);
                            wp.score += 1.0;
                        } else {
                            wp.losses = wp.losses.saturating_add(1);
                            wp.score -= 1.0;
                        }
                    }
                }
            } else {
                // If coin disappeared, mark as LOSS so it doesn't hang forever
                call.outcome = Some("LOSS".to_string());
            }
        }
    }
}
