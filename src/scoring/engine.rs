use crate::config::Config;
use crate::fmt::fmt_f64_0_commas;
use crate::market::cache::{market_trend, MarketCache};
use crate::missed_calls::MissedCallTracker;
use crate::reputation::{RUG_WALLETS, WALLET_REPUTATION};
use crate::rugcheck::{fetch_rug_report, RugReport};
use crate::scoring::shadow::{shadow_should_add, shadow_touch, ShadowMap};
use crate::scoring::window::{prune_window, window_wallets, window_whales};
use crate::time::now_ts;
use crate::types::{CallRecord, CoinState, WhaleTier};

use colored::*;
use std::collections::{HashMap, VecDeque};
use std::time::Instant;

// Cache rugcheck results so we don't hammer the API
lazy_static::lazy_static! {
    static ref RUG_CACHE: std::sync::Mutex<HashMap<String, (RugReport, u64)>> =
        std::sync::Mutex::new(HashMap::new());
}

const RUG_CACHE_TTL: u64 = 300; // re-fetch every 5 min

async fn get_rug_report(mint: &str) -> RugReport {
    let now = now_ts();
    {
        let cache = RUG_CACHE.lock().unwrap();
        if let Some((report, ts)) = cache.get(mint) {
            if now - ts < RUG_CACHE_TTL {
                return report.clone();
            }
        }
    }
    let report = fetch_rug_report(mint).await;
    {
        let mut cache = RUG_CACHE.lock().unwrap();
        cache.insert(mint.to_string(), (report.clone(), now));
    }
    report
}

pub async fn score_and_manage(
    cfg: &Config,
    coins: &mut HashMap<String, CoinState>,
    active: &mut Vec<String>,
    queue: &mut VecDeque<String>,
    calls: &mut Vec<CallRecord>,
    market: &MarketCache,
    shadow: &mut ShadowMap,
    missed: &mut MissedCallTracker,
) {
    let mints: Vec<String> = coins.keys().cloned().collect();
    let mut scanned = 0u64;
    let mut called = 0u64;
    let mut skip_bonk = 0u64;
    let mut skip_age = 0u64;
    let mut skip_no_market = 0u64;
    let mut skip_fdv_band = 0u64;
    let mut skip_velocity = 0u64;
    let mut skip_liq = 0u64;
    let mut skip_bsr = 0u64;
    let mut skip_rug = 0u64;
    let mut skip_activity = 0u64;
    let mut skip_active_full = 0u64;

    for mint in mints {
        scanned += 1;

        if cfg.avoid_bonk && (mint.to_lowercase().contains("bonk")) {
            skip_bonk += 1;
            continue;
        }

        let Some(c) = coins.get_mut(&mint) else { continue; };

        prune_window(&mut c.events, cfg.events_keep_secs);

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

        let trend = market_trend(market, &mint, cfg);
        let fdv = match trend.last_fdv {
            Some(f) if f > 0.0 => f,
            _ => { skip_no_market += 1; continue; }
        };
        let liq = trend.last_liq.unwrap_or(0.0);

        // Track for missed call analysis
        let was_called = calls.iter().any(|c| c.mint == mint);
        missed.update(&mint, fdv, trend.buys_5m, trend.fdv_velocity_pct, trend.buy_sell_ratio, cfg, was_called);

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

        // Liquidity hard gate
        if liq < 3_000.0 {
            skip_liq += 1;
            continue;
        }

        // BSR hard gate — no net selling
        // Exception: if FDV velocity is strong (≥15%/min) or coin is very new (<5min),
        // allow BSR < 1.0 — high-volume pumpswap rockets often have sell-heavy tx counts
        // while still mooning because buy SIZE dominates. Don't kill them on tx count alone.
        let total_tx = trend.buys_5m + trend.sells_5m;
        let bsr = if total_tx >= 5 { trend.buy_sell_ratio } else { 1.0 };
        let coin_age_secs = c.first_seen.elapsed().as_secs();
        let bsr_bypass = trend.fdv_velocity_pct >= 15.0 || coin_age_secs < 300;
        if bsr < 1.0 && total_tx >= 5 && !bsr_bypass {
            skip_bsr += 1;
            continue;
        }

        // ── THE LAB BOOST ─────────────────────────────────────────────
        // Check if this mint was sourced from THE LAB — give it +20 conviction
        let lab_boost = {
            let seed_path = "data/lab_seeds.json";
            if let Ok(s) = std::fs::read_to_string(seed_path) {
                if let Ok(seeds) = serde_json::from_str::<Vec<String>>(&s) {
                    seeds.contains(&mint)
                } else { false }
            } else { false }
        };

        // ── LATE ENTRY CHECK (hard gate) ──────────────────────────────
        if trend.late_entry {
            skip_velocity += 1;
            continue; // coin already peaked, don't chase
        }

        let vel = trend.fdv_velocity_pct; // keep for near-miss logging

        // ── LANE CLASSIFICATION (from psychic-spoon) ──────────────────
        let lane = if trend.early_snipe {
            "SNIPE"
        } else if trend.conviction_momentum {
            "CONVICTION"
        } else if trend.buys_1h >= 20 && trend.price_change_1h.abs() > 5.0 {
            "SNIPE" // slow SNIPE — already moving over 1h
        } else {
            "NEWBORN"
        };

        // ── SCORE ─────────────────────────────────────────────────────
        let mut score = 0i32;

        // 1. Primary signal — SNIPE/CONVICTION from psychic-spoon logic
        if trend.early_snipe {
            // FDV < $50k + 15%+ growth in 5m = strong early signal
            score += 40;
            score += (trend.fdv_growth_5m_pct * 2.0).min(40.0) as i32;
        } else if trend.conviction_momentum {
            // $15k+ abs gain = real money in
            score += 30;
            score += (trend.fdv_abs_gain_5m / 1000.0).min(30.0) as i32;
        } else {
            // Fall back to velocity for other coins
            let vel = trend.fdv_velocity_pct;
            if vel < -2.0 { skip_velocity += 1; continue; }
            if vel > 0.0 { score += (vel * 6.0).min(40.0) as i32; }
        }

        // 2. Buy pressure 5m
        let total_5m = trend.buys_5m + trend.sells_5m;
        if total_5m < cfg.min_buys_5m as u64 {
            // SNIPE/CONVICTION can pass with less activity
            if lane == "NEWBORN" { skip_activity += 1; continue; }
        }
        if bsr >= 1.5 { score += 10; }
        if bsr >= 2.0 { score += 10; }
        if bsr >= 3.0 { score += 10; }
        if trend.buys_5m >= 25 { score += 10; }
        if trend.buys_5m >= 50 { score += 15; }

        // 3. Slow climber — 1h signals
        if trend.buys_1h >= 50 && trend.bsr_1h >= 1.3 { score += 15; }
        if trend.price_change_1h > 10.0 { score += 10; }
        if trend.price_change_1h > 25.0 { score += 10; }
        if trend.volume_1h > 50_000.0 { score += 8; }

        // 4. Liquidity quality
        if liq >= 10_000.0 { score += 5; }
        if liq >= 30_000.0 { score += 10; }
        if trend.liq_velocity_pct > 1.0 { score += 10; }

        // 5. Real buy size from Helius (0.5+ SOL = real human)
        let recent_ts = now_ts();
        let beluga_count = c.events.iter()
            .filter(|e| recent_ts.saturating_sub(e.ts) < cfg.window_secs)
            .filter(|e| e.tier == WhaleTier::Beluga || e.tier == WhaleTier::Blue)
            .count();
        let blue_count = c.events.iter()
            .filter(|e| recent_ts.saturating_sub(e.ts) < cfg.window_secs)
            .filter(|e| e.tier == WhaleTier::Blue)
            .count();
        score += (beluga_count as i32) * 8;
        score += (blue_count as i32) * 15;

        // 6. Wallet reputation modifier
        let wallets = window_wallets(&c.events, cfg.window_secs);
        if !wallets.is_empty() {
            let rep_lock = WALLET_REPUTATION.lock().unwrap();
            let rug_lock = RUG_WALLETS.lock().unwrap();
            let mut bad_count = 0usize;
            let mut good_boost = 0i32;
            for w in &wallets {
                if rug_lock.contains(w) { bad_count += 1; }
                else if let Some(rep) = rep_lock.get(w) {
                    if *rep > 10.0 { good_boost += 8; }
                    else if *rep > 5.0 { good_boost += 4; }
                    else if *rep < -5.0 { bad_count += 1; }
                }
            }
            let bad_ratio = bad_count as f64 / wallets.len() as f64;
            if bad_ratio > 0.20 {
                skip_rug += 1;
                continue;
            }
            score += good_boost.min(20);
        }

        // Apply LAB boost
        if lab_boost {
            score += 20;
            println!("{}", format!("🧪 LAB BOOST +20 → {}", &mint[..12]).cyan());
        }

        let is_snipe = lane == "SNIPE" || lane == "CONVICTION";

        // 8. Shadow watch for near-misses
        if shadow_should_add(score, cfg, if trend.fdv_accel { 1.0 } else { 0.0 }, 0.0) {
            shadow_touch(shadow, &mint, cfg, score);
        }

        // Near-miss log
        if score >= cfg.score_target - 20 && score < cfg.score_target {
            println!(
                "{}",
                format!(
                    "👀 WATCH {} | FDV ${} | vel {:.1}%/min | BSR {:.1}x | buys {} | score {}",
                    &mint[..8], fmt_f64_0_commas(fdv), vel, bsr, trend.buys_5m, score
                ).bright_black()
            );
            continue;
        }

        if score < cfg.score_target {
            skip_activity += 1;
            continue;
        }

        // ── RUGCHECK (only run on coins that pass score) ──────────────
        let rug = get_rug_report(&mint).await;
        if rug.fetched {
            if !rug.is_safe() {
                println!(
                    "{}",
                    format!(
                        "🚫 RUG BLOCKED {} | score={} holders={} top1={:.0}% mint_revoked={} risks={:?}",
                        &mint[..12], rug.score, rug.total_holders,
                        rug.top_holder_pct, rug.mint_authority_revoked,
                        rug.risks.iter().take(2).collect::<Vec<_>>()
                    ).red().bold()
                );
                skip_rug += 1;
                continue;
            }
            score += rug.score_modifier();
        }

        // ── CALL ──────────────────────────────────────────────────────
        // Never call the same mint twice
        if calls.iter().any(|existing| existing.mint == mint) {
            continue;
        }

        if c.active {
            // Demotion
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

            println!(
                "{}",
                format!(
                    "🎯 {} → {} | FDV ${} | LIQ ${} | 5m {:.0}% (+${:.0}) | 1h {:.0}% | BSR {:.1}x | b5m {} | b1h {} | holders {} | top1 {:.0}% | score {}",
                    lane,
                    mint.green().bold(),
                    fmt_f64_0_commas(fdv).cyan(),
                    fmt_f64_0_commas(liq).cyan(),
                    trend.fdv_growth_5m_pct,
                    trend.fdv_abs_gain_5m,
                    trend.price_change_1h,
                    bsr,
                    trend.buys_5m,
                    trend.buys_1h,
                    rug.total_holders,
                    rug.top_holder_pct,
                    score.to_string().yellow().bold(),
                ).bold().green()
            );

            let wallets_involved = window_wallets(&c.events, cfg.window_secs);
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
    }

    println!(
        "{}",
        format!(
            "DBG scanned={} called={} skip(bonk={} age={} nodata={} fdv={} vel={} liq={} bsr={} rug={} activity={} full={})",
            scanned, called, skip_bonk, skip_age, skip_no_market,
            skip_fdv_band, skip_velocity, skip_liq, skip_bsr, skip_rug, skip_activity, skip_active_full
        ).bright_black()
    );
}
