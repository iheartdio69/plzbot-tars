#!/usr/bin/env bash
set -euo pipefail

# Run from project root (where Cargo.toml is)
ROOT="$(pwd)"

echo "==> Backing up existing scoring/mod.rs (if any)"
if [ -f "src/scoring/mod.rs" ]; then
  cp -v "src/scoring/mod.rs" "src/scoring/mod.rs.bak.$(date +%s)" || true
fi

echo "==> Creating modular engine folder + files"
mkdir -p src/scoring/engine

# -------------------------------
# src/scoring/mod.rs
# -------------------------------
cat > src/scoring/mod.rs <<'RS'
pub mod engine;

// keep existing scoring modules available at crate::scoring::*
pub mod shadow;
pub mod onchain;
pub mod wallet_rep;
pub mod call_outcomes;
pub mod wallet_outcomes;
pub mod window;
RS

# -------------------------------
# src/scoring/engine/mod.rs
# -------------------------------
cat > src/scoring/engine/mod.rs <<'RS'
mod counters;
mod constants;
mod score;
mod active;
mod queue;
mod calls;
mod summary;

pub use counters::Counters;
pub use constants::*;
pub use score::score_all_coins;
pub use active::{update_active_list, promote_from_queue, remove_shadowed_after_calls};
pub use queue::fill_queue;
pub use calls::process_calls;
pub use summary::print_summary;

use crate::config::Config;
use crate::db::Db;
use crate::market::cache::MarketCache;
use crate::scoring::shadow;
use crate::types::{CallRecord, CoinState};

use std::collections::{HashMap, VecDeque};

pub fn score_and_manage(
    cfg: &Config,
    coins: &mut HashMap<String, CoinState>,
    active: &mut Vec<String>,
    queue: &mut VecDeque<String>,
    calls: &mut Vec<CallRecord>,
    market: &MarketCache,
    shadow_map: &mut shadow::ShadowMap,
    db: &mut Db,
) {
    let now = crate::time::now();
    let now_ts = now as i64;

    let mut counters = Counters::default();

    // 1) score + per-coin maintenance
    score_all_coins(
        cfg,
        coins,
        market,
        db,
        shadow_map,
        now,
        now_ts,
        &mut counters,
        active,
    );

    // 2) clean active list
    update_active_list(cfg, coins, active, shadow_map, now);

    // 3) fill queue
    fill_queue(cfg, coins, active, queue, market, shadow_map, &mut counters);

    // 4) promote from queue
    promote_from_queue(cfg, coins, active, queue, market, shadow_map, now, &mut counters);

    // 5) process calls (returns mints shadowed-after-call)
    let shadowed_after_call = process_calls(
        cfg,
        coins,
        active,
        calls,
        market,
        shadow_map,
        db,
        now,
        now_ts,
        &mut counters,
    );

    // 6) remove shadowed mints from active
    remove_shadowed_after_calls(active, &shadowed_after_call);

    // 7) summary
    print_summary(&counters, active.len(), queue.len());
}
RS

# -------------------------------
# src/scoring/engine/counters.rs
# -------------------------------
cat > src/scoring/engine/counters.rs <<'RS'
#[derive(Debug, Default, Clone)]
pub struct Counters {
    pub considered: usize,
    pub called: usize,

    pub skipped_cooldown: usize,
    pub skipped_threshold: usize,

    pub skip_fdv: usize,
    pub skip_conc: usize,
    pub skip_wallet: usize,
    pub skip_signer: usize,
    pub skip_cooldown: usize,
    pub skip_other: usize,

    pub queue_dropped_ttl: usize,
}
RS

# -------------------------------
# src/scoring/engine/constants.rs
# -------------------------------
cat > src/scoring/engine/constants.rs <<'RS'
pub const ACTIVE_TTL_SEC: u64 = 600;      // 10 min max active
pub const NO_PROGRESS_TTL_SEC: u64 = 120; // 2 min no new events
pub const QUEUE_TTL_SEC: u64 = 900;       // 15 min max in queue
RS

# -------------------------------
# src/scoring/engine/summary.rs
# -------------------------------
cat > src/scoring/engine/summary.rs <<'RS'
use super::Counters;

pub fn print_summary(c: &Counters, active_len: usize, queue_len: usize) {
    println!(
        "{}🧮 scoring{} considered={} called={} skips: fdv={} conc={} wallet={} cooldown={} signer={} other={} queue_dropped={} active={} queue={}",
        crate::fmt::YELLOW,
        crate::fmt::RESET,
        c.considered,
        c.called,
        c.skip_fdv,
        c.skip_conc,
        c.skip_wallet,
        c.skip_cooldown,
        c.skip_signer,
        c.skip_other,
        c.queue_dropped_ttl,
        active_len,
        queue_len
    );
}
RS

# -------------------------------
# src/scoring/engine/active.rs
# -------------------------------
cat > src/scoring/engine/active.rs <<'RS'
use super::{Counters, QUEUE_TTL_SEC};
use crate::config::Config;
use crate::market::cache::MarketCache;
use crate::scoring::shadow;
use crate::types::CoinState;
use std::collections::{HashMap, VecDeque};

pub fn update_active_list(
    cfg: &Config,
    coins: &mut HashMap<String, CoinState>,
    active: &mut Vec<String>,
    shadow_map: &mut shadow::ShadowMap,
    now: u64,
) {
    // Keep same behavior as old engine: drop demoted OR inactive, shadow briefly.
    active.retain(|mint| {
        if let Some(st) = coins.get(mint) {
            if u32::from(st.low_score_streak) >= cfg.demote_streak || !st.active {
                shadow::shadow_for(shadow_map, mint, now, 120);
                false
            } else {
                true
            }
        } else {
            false
        }
    });
}

pub fn promote_from_queue(
    cfg: &Config,
    coins: &mut HashMap<String, CoinState>,
    active: &mut Vec<String>,
    queue: &mut VecDeque<String>,
    market: &MarketCache,
    shadow_map: &mut shadow::ShadowMap,
    now: u64,
    counters: &mut Counters,
) {
    while active.len() < cfg.max_active_coins {
        let Some(mint) = queue.pop_front() else { break };

        // TTL drop
        if let Some(st) = coins.get(&mint) {
            if st.queued_since > 0 && (now - st.queued_since) > QUEUE_TTL_SEC {
                counters.queue_dropped_ttl += 1;
                continue;
            }
        }

        // Revalidate before promote
        if let Some(st) = coins.get(&mint) {
            if st.score < 20 || shadow::is_shadowed(shadow_map, mint.as_str(), now) {
                continue;
            }
        } else {
            continue;
        }

        let _ = market; // keep signature stable for later use/logging

        active.push(mint.clone());
        if let Some(st) = coins.get_mut(&mint) {
            st.active = true;
            st.active_since = now;
        }

        println!(
            "{}",
            crate::fmt::active_line(&mint, coins.get(&mint).map(|s| s.score).unwrap_or(0))
        );
    }
}

pub fn remove_shadowed_after_calls(active: &mut Vec<String>, shadowed: &[String]) {
    for m in shadowed {
        active.retain(|x| x != m);
    }
}
RS

# -------------------------------
# src/scoring/engine/queue.rs
# -------------------------------
cat > src/scoring/engine/queue.rs <<'RS'
use super::Counters;
use crate::config::Config;
use crate::market::cache::MarketCache;
use crate::scoring::shadow;
use crate::types::CoinState;
use std::collections::{HashMap, VecDeque};

pub fn fill_queue(
    _cfg: &Config,
    coins: &mut HashMap<String, CoinState>,
    active: &Vec<String>,
    queue: &mut VecDeque<String>,
    market: &MarketCache,
    shadow_map: &shadow::ShadowMap,
    counters: &mut Counters,
) {
    // Borrow-safe enqueue pattern
    let now = crate::time::now();

    let mut to_enqueue: Vec<String> = Vec::new();
    for (mint, st) in coins.iter() {
        if st.score >= 20
            && !active.contains(mint)
            && !queue.contains(mint)
            && !shadow::is_shadowed(shadow_map, mint.as_str(), now)
        {
            to_enqueue.push(mint.clone());
        }
    }

    for mint in to_enqueue {
        // stamp queued_since
        if let Some(st) = coins.get_mut(&mint) {
            st.queued_since = now;
        }

        let _fdv_dbg = market.map.get(&mint).and_then(|ms| ms.fdv).unwrap_or(0.0);
        queue.push_back(mint);
        counters.considered += 0; // no-op; keeps counters in scope for later enhancements
    }
}
RS

# -------------------------------
# src/scoring/engine/score.rs
# -------------------------------
cat > src/scoring/engine/score.rs <<'RS'
use super::{Counters, ACTIVE_TTL_SEC, NO_PROGRESS_TTL_SEC};
use crate::config::Config;
use crate::market::cache::MarketCache;
use crate::scoring::shadow;
use crate::types::{CoinState, WhaleTier};
use std::collections::HashMap;

pub fn score_all_coins(
    cfg: &Config,
    coins: &mut HashMap<String, CoinState>,
    market: &MarketCache,
    db: &mut crate::db::Db,
    shadow_map: &mut shadow::ShadowMap,
    now: u64,
    now_ts: i64,
    counters: &mut Counters,
    active: &Vec<String>,
) {
    for (mint, st) in coins.iter_mut() {
        counters.considered += 1;

        let Some(ms) = market.map.get(mint) else {
            st.score = 0;
            st.wallet_delta = 0;
            continue;
        };

        let fdv = ms.fdv.unwrap_or(0.0);
        let liq = ms.liq.unwrap_or(0.0);
        st.tx_5m = ms.tx_5m.unwrap_or(0) as usize;

        // keep last 20m of events
        let keep_since = now.saturating_sub(1200);
        st.events.retain(|e| e.ts >= keep_since);

        st.unique_signers_5m = db.signers_5m(now_ts, mint.as_str()).unwrap_or(0) as usize;

        // quality scan last 5m
        let mut quality_hits: i64 = 0;
        let mut quality_sum: i64 = 0;
        let cutoff = now.saturating_sub(300);
        let mut sampled = 0usize;

        for e in st.events.iter().rev() {
            if e.ts < cutoff { break; }
            sampled += 1;
            if sampled > 200 { break; }

            let sc = db.wallet_score(&e.wallet).unwrap_or(0);
            if sc >= 10 {
                quality_hits += 1;
                quality_sum += sc.min(500);
            }
        }

        // cache wallet_delta ONCE per tick (call loop reads it)
        let early_fdv = fdv < 25_000.0;
        st.wallet_delta =
            if early_fdv && quality_hits < 2 && quality_sum < 80 && st.unique_signers_5m < 18 {
                -40
            } else {
                0
            };

        // reset conc flag each tick (optional but you want visibility)
        st.skip_call_for_conc = false;

        // score calc (same as your old engine)
        let mut score: i32 = 0;

        if fdv >= cfg.min_watch_fdv_usd && fdv <= cfg.max_watch_fdv_usd { score += 15; }
        if liq >= cfg.min_liq_usd { score += 15; }

        if let Some(pct) = db.fdv_change_pct(mint.as_str(), now_ts, 300) {
            if pct >= 0.30 { score += 15; }
            else if pct >= 0.20 { score += 10; }
            else if pct >= 0.10 { score += 5; }
        }

        let fdv_delta_30s = db.fdv_delta_recent(mint.as_str(), now_ts, 30).unwrap_or(0.0);
        if fdv_delta_30s >= 50_000.0 { score += 25; }
        else if fdv_delta_30s >= 25_000.0 { score += 15; }
        else if fdv_delta_30s >= 10_000.0 { score += 10; }

        if st.tx_5m >= 5 { score += 15; }

        if !st.events.is_empty() {
            match st.unique_signers_5m {
                0 => score -= 5,
                1..=2 => score += 5,
                3..=5 => score += 10,
                _ => score += 20,
            }
        }

        if quality_hits >= 2 { score += 10; }
        if quality_hits >= 5 { score += 10; }
        if quality_sum >= 250 { score += 10; }

        // whales last 5m
        let whale_cutoff = now.saturating_sub(300);
        let mut beluga: i32 = 0;
        let mut blue: i32 = 0;
        for e in st.events.iter().rev() {
            if e.ts < whale_cutoff { break; }
            match e.tier {
                WhaleTier::Beluga => beluga += 1,
                WhaleTier::Blue => blue += 1,
                WhaleTier::None => {}
            }
        }
        score += beluga * 5;
        score += blue * 10;

        if shadow::is_shadowed(shadow_map, mint.as_str(), now) { score -= 50; }

        st.score = score;

        // demotion streak
        if st.score < cfg.score_demote { st.low_score_streak = st.low_score_streak.saturating_add(1); }
        else { st.low_score_streak = 0; }

        // active rotation TTL
        if st.active && (now - st.active_since) > ACTIVE_TTL_SEC { st.active = false; }

        // no-progress deactivation
        let last_event_ts = st.events.last().map(|e| e.ts).unwrap_or(0);
        if st.active && (now - last_event_ts) > NO_PROGRESS_TTL_SEC { st.active = false; }

        let _ = db.insert_snapshot(
            now_ts,
            mint,
            Some(fdv),
            ms.tx_5m,
            st.score,
            st.unique_signers_5m as u64,
            st.events.len(),
            st.first_seen,
            active.contains(mint),
            false,
        );
    }
}
RS

# -------------------------------
# src/scoring/engine/calls.rs
# -------------------------------
cat > src/scoring/engine/calls.rs <<'RS'
use super::Counters;
use crate::config::Config;
use crate::market::cache::MarketCache;
use crate::scoring::shadow;
use crate::types::{CallRecord, CoinState};
use std::collections::HashMap;

pub fn process_calls(
    _cfg: &Config,
    coins: &mut HashMap<String, CoinState>,
    active: &mut Vec<String>,
    calls: &mut Vec<CallRecord>,
    market: &MarketCache,
    shadow_map: &mut shadow::ShadowMap,
    db: &mut crate::db::Db,
    now: u64,
    now_ts: i64,
    counters: &mut Counters,
) -> Vec<String> {
    let active_snapshot = active.clone();
    let mut shadowed_after_call: Vec<String> = Vec::new();

    for mint in active_snapshot.iter() {
        let Some(ms) = market.map.get(mint) else { continue };

        let fdv = ms.fdv.unwrap_or(0.0);
        let tx5: u64 = ms.tx_5m.unwrap_or(0);

        let signers_u: usize = coins.get(mint).map(|s| s.unique_signers_5m).unwrap_or(0);
        let signers: u64 = signers_u as u64;

        let ev: usize = db.events_5m(now_ts, mint.as_str()).unwrap_or(0) as usize;

        let mut uniq_sigs: u64 = db.uniq_sigs_5m(now_ts, mint.as_str()).unwrap_or(0);
        if uniq_sigs == 0 && tx5 > 0 {
            uniq_sigs = (tx5 / 3).max(1);
        }
        let signer_strength: u64 = std::cmp::max(signers, uniq_sigs);

        let first_seen: u64 = coins.get(mint).map(|s| s.first_seen).unwrap_or(0);
        let age_sec: u64 = now.saturating_sub(first_seen);

        // ----------------------------
        // Gate 1: FDV bounds
        // ----------------------------
        if fdv < 5_000.0 || fdv > 1_000_000.0 {
            counters.skip_fdv += 1;
            counters.skipped_threshold += 1;
            continue;
        }

        let gambol_ok = (signers == 0) && (tx5 >= 200);

        // ----------------------------
        // Gate 2: spam guards
        // ----------------------------
        if signers == 0 && tx5 < 50 && signer_strength < 8 {
            counters.skip_signer += 1;
            counters.skipped_threshold += 1;
            continue;
        }

        let tx_spam = tx5 >= 200 && signers == 0 && ev == 0;
        let low_signal = tx5 >= 80 && signers == 0 && ev == 0;
        if tx_spam || low_signal {
            counters.skip_other += 1;
            counters.skipped_threshold += 1;
            continue;
        }

        // ----------------------------
        // Gate 3: wallet gate (use cached wallet_delta)
        // ----------------------------
        let wallet_delta = coins.get(mint).map(|s| s.wallet_delta).unwrap_or(0);
        if wallet_delta < 0 {
            counters.skip_wallet += 1;
            counters.skipped_threshold += 1;
            continue;
        }

        // ----------------------------
        // Gate 4: onchain signal
        // ----------------------------
        let onchain_ok = (signers > 0) || (ev > 0) || (signer_strength >= 12);
        if !onchain_ok && !gambol_ok {
            counters.skip_other += 1;
            counters.skipped_threshold += 1;
            continue;
        }

        // ----------------------------
        // Gate 5: age
        // ----------------------------
        let max_age_sec: u64 = 2 * 60 * 60;
        let revival_ok: bool = signer_strength >= 25;
        if age_sec > max_age_sec && !revival_ok {
            counters.skip_other += 1;
            counters.skipped_threshold += 1;
            continue;
        }

        if !gambol_ok && signer_strength < 10 {
            counters.skip_signer += 1;
            counters.skipped_threshold += 1;
            continue;
        }

        // ----------------------------
        // Gate 6: cooldown
        // ----------------------------
        let cooldown_secs: i64 = 15 * 60;
        if let Ok(Some(last_ts)) = db.last_call_ts_for_mint(mint.as_str()) {
            if now_ts - last_ts < cooldown_secs {
                counters.skipped_cooldown += 1;
                counters.skip_cooldown += 1;
                continue;
            }
        }

        // ----------------------------
        // Gate 7: concentration PRE-CALL (and set flag for visibility)
        // ----------------------------
        let call_ts = now_ts;
        let start_ts = call_ts - 300;
        let end_ts = call_ts;

        let mut conc_risk = false;
        if let Ok(top) = db.top_wallets_for_mint_window(mint, start_ts, end_ts, 25) {
            let total_edges_all: i64 = db
                .total_edges_for_mint_window(mint, start_ts, end_ts)
                .unwrap_or(0);

            if total_edges_all > 0 {
                let top1_edges = top.get(0).map(|(_, e)| *e).unwrap_or(0);
                let top5_edges: i64 = top.iter().take(5).map(|(_, e)| *e).sum();

                let top1 = top1_edges as f64 / total_edges_all as f64;
                let top5 = top5_edges as f64 / total_edges_all as f64;

                conc_risk = top1 >= 0.22 || top5 >= 0.55;
            }
        }

        if conc_risk {
            if let Some(st) = coins.get_mut(mint) {
                st.skip_call_for_conc = true;
            }
            counters.skip_conc += 1;
            counters.skipped_threshold += 1;
            continue;
        }

        // ----------------------------
        // ACCEPT CALL
        // ----------------------------
        let base_score = coins.get(mint).map(|s| s.score).unwrap_or(0);
        let effective_score = base_score + wallet_delta;

        calls.push(CallRecord { mint: mint.clone(), ts: now, score: effective_score });
        counters.called += 1;

        let term_line = if gambol_ok {
            crate::fmt::red(&crate::fmt::call_line(
                mint.as_str(), fdv, effective_score, tx5, signers as usize, ev,
            ))
        } else {
            crate::fmt::call_line(mint.as_str(), fdv, effective_score, tx5, signers as usize, ev)
        };
        println!("{}", term_line);

        let tag = if gambol_ok { Some("GAMBOL") } else { None };

        if let Err(e) = db.insert_call(call_ts, mint, fdv, effective_score, tx5, signers, ev, tag) {
            eprintln!("DBG db insert_call failed mint={} err={:?}", crate::fmt::mint(mint), e);
            continue;
        }

        let first_seen: u64 = coins.get(mint).map(|s| s.first_seen).unwrap_or(0);
        let _ = db.insert_snapshot(
            call_ts,
            mint,
            Some(fdv),
            ms.tx_5m,
            effective_score,
            signers,
            ev,
            first_seen,
            true,
            true,
        );

        let log = format!("{},{},{:.0},{},{},{}", call_ts, mint, fdv, effective_score, tx5, signers);
        crate::call_log::append_call_line(&log);

        shadow::shadow_for(shadow_map, mint.as_str(), now, 180);
        shadowed_after_call.push(mint.clone());
        if let Some(st) = coins.get_mut(mint) {
            st.active = false;
        }
    }

    shadowed_after_call
}
RS

