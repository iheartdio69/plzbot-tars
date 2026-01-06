// scoring/engine.rs
use crate::config::Config;
use crate::market::cache::{market_trend, MarketCache};
use crate::scoring::shadow::{shadow_should_add, shadow_touch, ShadowMap};
use crate::scoring::window::{
    prune_window, runner_score, window_stats_for, window_wallets, window_whales,
};
use crate::types::{CallRecord, CoinState};
use crate::time::now_ts; // assuming you have this helper
use crate::fmt::fmt_f64_0_commas;

use colored::*;
use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Instant;

use lazy_static::lazy_static;
use std::sync::Mutex;

lazy_static! {
    static ref WALLET_REPUTATION: Mutex<HashMap<String, f64>> = Mutex::new(HashMap::new());
    static ref RUG_WALLETS: Mutex<HashSet<String>> = Mutex::new(HashSet::new());
}

fn is_bonk_like(s: &str) -> bool {
    let x = s.to_lowercase();
    x.contains("bonk") || x.ends_with("bonk")
}

#[derive(Debug, Default)]
struct SkipCounters {
    scanned: u64,
    called: u64,

    skip_bonk: u64,
    skip_missing_coin: u64,
    skip_snapshot: u64,
    skip_age: u64,
    skip_no_market: u64,
    skip_watch_band: u64,
    skip_call_band: u64,
    skip_liq: u64,
    skip_young_baseline: u64,
    skip_activity: u64,
    skip_active_full: u64,
    // you can add more if needed
}

impl SkipCounters {
    fn print_summary(&self) {
        println!(
            "{}",
            format!(
                "DBG summary scanned={} called={} skip(bonk={} age={} no_market={} watch={} callband={} liq={} young={} activity={} active_full={})",
                self.scanned, self.called, self.skip_bonk, self.skip_age, self.skip_no_market,
                self.skip_watch_band, self.skip_call_band, self.skip_liq, self.skip_young_baseline,
                self.skip_activity, self.skip_active_full
            )
            .bright_black()
        );
    }
}

pub fn score_and_manage(
    cfg: &Config,
    coins: &mut HashMap<String, CoinState>,
    active: &mut Vec<String>,
    queue: &mut VecDeque<String>,
    calls: &mut Vec<CallRecord>,
    market: &MarketCache,
    shadow: &mut ShadowMap,
) {
    let mints: Vec<String> = coins.keys().cloned().collect();
    let mut sk = SkipCounters::default();

    for mint in mints {
        sk.scanned += 1;

        if cfg.avoid_bonk && is_bonk_like(&mint) {
            sk.skip_bonk += 1;
            continue;
        }

        let Some(c) = coins.get_mut(&mint) else {
            sk.skip_missing_coin += 1;
            continue;
        };

        prune_window(&mut c.events, cfg.events_keep_secs);

        if !c.first_snapshot_done {
            c.first_snapshot_done = true;
        } else if c.last_snapshot.elapsed().as_secs() < cfg.snapshot_interval_secs {
            sk.skip_snapshot += 1;
            continue;
        }
        c.last_snapshot = Instant::now();

        let age_secs = c.first_seen.elapsed().as_secs();
        if age_secs < cfg.min_scan_age_secs || age_secs > cfg.max_coin_age_secs {
            sk.skip_age += 1;
            continue;
        }

        let trend = market_trend(market, &mint, cfg);
        let (fdv, liq) = match (trend.last_fdv, trend.last_liq) {
            (Some(fdv), Some(liq)) => (fdv, liq),
            _ => {
                sk.skip_no_market += 1;
                continue;
            }
        };

        if fdv < cfg.min_watch_fdv_usd || fdv > cfg.max_watch_fdv_usd {
            sk.skip_watch_band += 1;
            continue;
        }
        if liq < cfg.min_liq_usd {
            sk.skip_liq += 1;
            continue;
        }

        let (tx_now, signers_now, _whales_now) = window_stats_for(&c.events, cfg.window_secs);

        if age_secs < cfg.min_age_secs {
            c.prev_tx_window = tx_now;
            c.prev_signers_window = signers_now;
            sk.skip_young_baseline += 1;
            continue;
        }

        let (mut score, _wg, _tg) =
            runner_score(signers_now, tx_now, c.prev_signers_window, c.prev_tx_window);

        c.prev_tx_window = tx_now;
        c.prev_signers_window = signers_now;

        // Absolute activity floor
        let activity_floor = if signers_now >= cfg.min_signers_for_target && tx_now >= cfg.min_tx_for_target {
            10
        } else {
            0
        };
        score += activity_floor;

        // === WALLET QUALITY SCORE ===
        let wallets = window_wallets(&c.events, cfg.window_secs);

        let rep_lock = WALLET_REPUTATION.lock().unwrap();
        let rug_lock = RUG_WALLETS.lock().unwrap();

        let mut total_rep = 0.0_f64;
        let mut good_count = 0;
        let mut bad_count = 0;
        let mut top_good: Vec<(String, f64)> = Vec::new();
        let mut top_bad: Vec<(String, f64)> = Vec::new();

        for wallet in &wallets {
            let rep = rep_lock.get(wallet).cloned().unwrap_or(0.0);
            total_rep += rep;

            if rep > 5.0 {
                good_count += 1;
                top_good.push((wallet.clone(), rep));
            } else if rep < -5.0 || rug_lock.contains(wallet) {
                bad_count += 1;
                top_bad.push((wallet.clone(), rep));
            }
        }

        let avg_rep = if !wallets.is_empty() { total_rep / wallets.len() as f64 } else { 0.0 };
        let wallet_quality_boost = (avg_rep * 2.0) as i32
            + (good_count as i32 * 5)
            - (bad_count as i32 * 10);

        score += wallet_quality_boost;

        // Hard red-flag gate
        let bad_ratio = if signers_now > 0 { bad_count as f64 / signers_now as f64 } else { 0.0 };
        if bad_ratio > 0.20 {
            println!(
                "{}",
                format!(
                    "🚩 SKIPPED {} — high rug risk (bad_ratio={:.2}, bad={}) top_bad={:?}",
                    mint.red().bold(),
                    bad_ratio,
                    bad_count,
                    top_bad.iter().take(3).map(|(w, _)| w.as_str()).collect::<Vec<_>>()
                )
                .bold()
                .red()
            );
            sk.skip_activity += 1;
            continue;
        }

        top_good.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap());
        top_bad.sort_by(|a, b| a.1.partial_cmp(&b.1).unwrap());
        // === END WALLET QUALITY ===

        // Market boosts
        let mut market_boost = 0;
        if fdv > 50_000.0 { market_boost += 5; }
        if fdv > 150_000.0 { market_boost += 10; }
        if fdv > 300_000.0 { market_boost += 15; }
        // Add accel when you have it
        score += market_boost;

        // Shadow watch
        if shadow_should_add(score, cfg, trend.price_accel, trend.fdv_accel) {
            shadow_touch(shadow, &mint, cfg, score);
        }

        // Call band gate
        let in_call_band = fdv >= cfg.min_call_fdv_usd && fdv <= cfg.max_call_fdv_usd;
        if !in_call_band {
            sk.skip_call_band += 1;
            continue;
        }

        let passes_activity = score >= cfg.score_target
            && signers_now >= cfg.min_signers_for_target
            && tx_now >= cfg.min_tx_for_target;

        if !passes_activity {
            sk.skip_activity += 1;
            continue;
        }

        // Final call gate
        if c.active {
            // already active, do nothing extra
        } else if active.len() >= cfg.max_active_coins {
            sk.skip_active_full += 1;
            queue.push_back(mint.clone());
            continue;
        } else {
            c.active = true;
            sk.called += 1;
            active.push(mint.clone());

            let wallet_summary = format!(
                "WalletQ: avg={:+.1} good={} bad={} topG={:?} topB={:?}",
                avg_rep,
                good_count,
                bad_count,
                top_good.iter().take(3).map(|(w, _)| w.as_str()).collect::<Vec<_>>(),
                top_bad.iter().take(3).map(|(w, _)| w.as_str()).collect::<Vec<_>>(),
            );

            println!(
                "{}",
                format!(
                    "🎯 TARGET → {} | FDV ${} | LIQ ${} | score {} | tx {} | wallets {} | {}",
                    mint.green().bold(),
                    fmt_f64_0_commas(fdv).cyan(),
                    fmt_f64_0_commas(liq).cyan(),
                    score.to_string().bold().yellow(),
                    tx_now,
                    signers_now,
                    wallet_summary.magenta().bold()
                )
                .bold()
                .green()
            );

            let wallets_involved = wallets;
            let whales_involved = window_whales(&c.events, cfg.window_secs);

            calls.push(CallRecord {
                mint: mint.clone(),
                call_ts: now_ts(),
                score,
                t5_ts: None,
                wallets_t5: None,
                tx_t5: None,
                t15_ts: None,
                wallets_t15: None,
                tx_t15: None,
                outcome: None,
                wallets_involved,
                whales_involved,
            });
        }

        // Demotion logic
        if c.active {
            if score < cfg.score_demote {
                c.low_score_streak = c.low_score_streak.saturating_add(1);
            } else {
                c.low_score_streak = 0;
            }

            if c.low_score_streak >= cfg.demote_streak as u32 {
                c.active = false;
                c.low_score_streak = 0;
                active.retain(|m| m != &mint);

                println!(
                    "{}",
                    format!("📉 DEMOTE → {} (score {})", mint, score)
                        .bold()
                        .yellow()
                );

                // Promote next from queue
                while let Some(next) = queue.pop_front() {
                    if active.contains(&next) {
                        continue;
                    }
                    if let Some(nc) = coins.get_mut(&next) {
                        nc.active = true;
                        active.push(next.clone());
                        println!("{}", format!("🧠 FOCUS ADD → {}", next).green().bold());
                        break;
                    }
                }
            }
        }

        if cfg.debug_every_n_scans > 0 && (sk.scanned % cfg.debug_every_n_scans == 0) {
            sk.print_summary();
        }
    }

    if cfg.debug_every_n_scans == 0 || sk.scanned > 0 {
        sk.print_summary();
    }
}