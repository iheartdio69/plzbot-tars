use crate::config::Config;
use crate::market::cache::{market_trend, MarketCache};
use crate::reputation::{RUG_WALLETS, WALLET_REPUTATION};
use crate::scoring::shadow::{shadow_should_add, shadow_touch, ShadowMap};
use crate::scoring::window::{prune_window, window_wallets, window_whales};
use crate::time::now_ts;
use crate::fmt::fmt_f64_0_commas;
use crate::types::{CallRecord, CoinState};

use colored::*;
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

fn is_bonk_like(s: &str) -> bool {
    let x = s.to_lowercase();
    x.contains("bonk") || x.ends_with("bonk")
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
    let mut scanned = 0u64;
    let mut called = 0u64;
    let mut skip_bonk = 0u64;
    let mut skip_age = 0u64;
    let mut skip_no_market = 0u64;
    let mut skip_fdv_band = 0u64;
    let mut skip_velocity = 0u64;
    let mut skip_activity = 0u64;
    let mut skip_rug = 0u64;
    let mut skip_active_full = 0u64;

    for mint in mints {
        scanned += 1;

        if cfg.avoid_bonk && is_bonk_like(&mint) {
            skip_bonk += 1;
            continue;
        }

        let Some(c) = coins.get_mut(&mint) else { continue; };

        prune_window(&mut c.events, cfg.events_keep_secs);

        // Snapshot throttle
        if !c.first_snapshot_done {
            c.first_snapshot_done = true;
        } else if c.last_snapshot.elapsed().as_secs() < cfg.snapshot_interval_secs {
            continue;
        }
        c.last_snapshot = Instant::now();

        let age_secs = c.first_seen.elapsed().as_secs();
        if age_secs > cfg.max_coin_age_secs {
            skip_age += 1;
            continue;
        }

        // ── MARKET DATA ──────────────────────────────────────────────
        let trend = market_trend(market, &mint, cfg);
        let fdv = match trend.last_fdv {
            Some(f) if f > 0.0 => f,
            _ => { skip_no_market += 1; continue; }
        };
        let liq = trend.last_liq.unwrap_or(0.0);

        // FDV band gate
        if fdv < cfg.min_call_fdv_usd || fdv > cfg.max_call_fdv_usd {
            skip_fdv_band += 1;
            continue;
        }

        // Need at least 2 snapshots for velocity
        if trend.snapshots < 2 {
            skip_velocity += 1;
            continue;
        }

        // ── SCORE CALCULATION ────────────────────────────────────────
        let mut score = 0i32;

        // 1. FDV velocity (primary signal) — % per minute
        //    2%/min = +20pts, 5%/min = +40pts, 10%/min = +60pts
        let vel = trend.fdv_velocity_pct;
        if vel > 0.0 {
            score += (vel * 6.0).min(60.0) as i32;
        } else if vel < -5.0 {
            // Dumping fast — skip
            skip_velocity += 1;
            continue;
        }

        // 2. Buy/sell ratio (strong signal)
        //    1.5x = +10, 2x = +20, 3x+ = +30
        let bsr = trend.buy_sell_ratio;
        if bsr >= 1.5 { score += 10; }
        if bsr >= 2.0 { score += 10; }
        if bsr >= 3.0 { score += 10; }

        // 3. Raw buy count in 5m
        if trend.buys_5m >= cfg.min_buys_5m { score += 10; }
        if trend.buys_5m >= 25 { score += 10; }
        if trend.buys_5m >= 50 { score += 10; }

        // 4. Liquidity health
        if liq >= 5_000.0 { score += 5; }
        if liq >= 15_000.0 { score += 5; }

        // 5. Liquidity growing (not just FDV)
        if trend.liq_velocity_pct > 1.0 { score += 10; }

        // 6. Wallet reputation modifier
        let wallets = window_wallets(&c.events, cfg.window_secs);
        if !wallets.is_empty() {
            let rep_lock = WALLET_REPUTATION.lock().unwrap();
            let rug_lock = RUG_WALLETS.lock().unwrap();

            let mut bad_count = 0usize;
            let mut good_boost = 0i32;

            for w in &wallets {
                if rug_lock.contains(w) {
                    bad_count += 1;
                } else if let Some(rep) = rep_lock.get(w) {
                    if *rep > 10.0 { good_boost += 8; }
                    else if *rep > 5.0 { good_boost += 4; }
                    else if *rep < -5.0 { bad_count += 1; }
                }
            }

            // Hard rug gate: >20% bad wallets = skip
            let bad_ratio = bad_count as f64 / wallets.len() as f64;
            if bad_ratio > 0.20 {
                println!("{}", format!("🚩 RUG RISK {} bad_ratio={:.0}%", &mint[..8], bad_ratio * 100.0).red());
                skip_rug += 1;
                continue;
            }

            score += good_boost.min(20); // cap rep boost at 20pts
        }

        // Shadow watch for near-misses
        if shadow_should_add(score, cfg, if trend.fdv_accel { 1.0 } else { 0.0 }, 0.0) {
            shadow_touch(shadow, &mint, cfg, score);
        }

        // ── CALL GATE ────────────────────────────────────────────────
        if score < cfg.score_target {
            skip_activity += 1;

            // Log interesting near-misses
            if score >= cfg.score_target - 15 && vel > 1.0 {
                println!(
                    "{}",
                    format!(
                        "👀 WATCH {} | FDV ${} | vel {:.1}%/min | BSR {:.1}x | buys {} | score {}",
                        &mint[..8], fmt_f64_0_commas(fdv), vel, bsr, trend.buys_5m, score
                    ).bright_black()
                );
            }
            continue;
        }

        // ── MAKE THE CALL ────────────────────────────────────────────
        if c.active {
            // Already called, update demotion streak
            if score < cfg.score_demote {
                c.low_score_streak = c.low_score_streak.saturating_add(1);
            } else {
                c.low_score_streak = 0;
            }
            if c.low_score_streak >= cfg.demote_streak as u32 {
                c.active = false;
                c.low_score_streak = 0;
                active.retain(|m| m != &mint);
                println!("{}", format!("📉 DEMOTE {} (score {})", &mint[..12], score).yellow());
                while let Some(next) = queue.pop_front() {
                    if !active.contains(&next) {
                        if let Some(nc) = coins.get_mut(&next) {
                            nc.active = true;
                            active.push(next.clone());
                            println!("{}", format!("🧠 PROMOTE → {}", &next[..12]).green());
                            break;
                        }
                    }
                }
            }
        } else if active.len() >= cfg.max_active_coins {
            skip_active_full += 1;
            queue.push_back(mint.clone());
        } else {
            c.active = true;
            called += 1;
            active.push(mint.clone());

            let whales_involved = window_whales(&c.events, cfg.window_secs);
            let wallets_involved = window_wallets(&c.events, cfg.window_secs);

            println!(
                "{}",
                format!(
                    "🎯 CALL → {} | FDV ${} | LIQ ${} | vel {:.1}%/min | BSR {:.1}x | buys {} | score {}",
                    mint.green().bold(),
                    fmt_f64_0_commas(fdv).cyan(),
                    fmt_f64_0_commas(liq).cyan(),
                    vel,
                    bsr,
                    trend.buys_5m,
                    score.to_string().yellow().bold(),
                )
                .bold()
                .green()
            );

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
    }

    println!(
        "{}",
        format!(
            "DBG scanned={} called={} skip(bonk={} age={} nodata={} fdv={} vel={} activity={} rug={} full={})",
            scanned, called, skip_bonk, skip_age, skip_no_market,
            skip_fdv_band, skip_velocity, skip_activity, skip_rug, skip_active_full
        ).bright_black()
    );
}
