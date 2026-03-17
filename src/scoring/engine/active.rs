// src/scoring/active.rs

use super::{Counters, QUEUE_TTL_SEC};
use crate::config::Config;
use crate::db::Db;
use crate::market::cache::MarketCache;
use crate::scoring::shadow;
use crate::types::CoinState;
use std::collections::{HashMap, HashSet, VecDeque};

fn debug_enabled() -> bool {
    std::env::var("DEBUG").ok().as_deref() == Some("1")
}

/// Drop demoted coins from `active`.
/// Shadow ONLY on demotion.
/// If a coin is inactive for any other reason, just remove it (no shadow),
/// otherwise you end up starving the queue.
pub fn update_active_list(
    cfg: &Config,
    coins: &mut HashMap<String, CoinState>,
    active: &mut Vec<String>,
    shadow_map: &mut shadow::ShadowMap,
    now: u64,
) {
    active.retain(|mint| {
        let Some(st) = coins.get(mint) else {
            return false;
        };

        // demotion based on low-score streak
        let demoted: bool = u32::from(st.low_score_streak) >= cfg.demote_streak.max(1);
        if demoted {
            shadow::shadow_for(shadow_map, mint.as_str(), now, 120);
            return false;
        }

        // If it's not active anymore, drop it without shadowing.
        st.active
    });
}

/// Rotate out idle actives to keep list fresh.
/// - avoids queue duplicates
/// - briefly shadows rotated coins to prevent immediate re-promote ping-pong
///
/// Rotation policy:
/// - Candidate if:
///   - active
///   - dwell >= min_dwell
///   - AND (idle >= idle_sec OR dwell >= max_dwell)
/// - Sorted by oldest last_activity_ts first, then lowest score
pub fn rotate_least_active(
    _cfg: &Config,
    coins: &mut HashMap<String, CoinState>,
    active: &mut Vec<String>,
    queue: &mut VecDeque<String>,
    shadow_map: &mut shadow::ShadowMap,
    now: u64,
    counters: &mut Counters,
) {
    // tunables
    let k: usize = 3;
    let min_dwell: u64 = 45;
    let idle_sec: u64 = 20;
    let max_dwell: u64 = 3 * 60;
    let rotate_shadow_secs: u64 = 45;

    if active.is_empty() {
        return;
    }

    // snapshot queued set so we don't duplicate
    let queued: HashSet<String> = queue.iter().cloned().collect();

    // Pick eligible (idle + had a chance, or force rotate)
    let mut eligible: Vec<(String, u64, i32)> = Vec::new();
    for mint in active.iter() {
        let Some(st) = coins.get(mint) else { continue };

        let dwell: u64 = now.saturating_sub(st.active_since);
        let idle: u64 = now.saturating_sub(st.last_activity_ts);
        let force_rotate: bool = dwell >= max_dwell;

        if st.active && dwell >= min_dwell && (idle >= idle_sec || force_rotate) {
            eligible.push((mint.clone(), st.last_activity_ts, st.score));
        }
    }

    if eligible.is_empty() {
        return;
    }

    // Oldest activity first, then lowest score
    eligible.sort_by(|a, b| a.1.cmp(&b.1).then(a.2.cmp(&b.2)));

    let mut rotated: usize = 0;

    for (mint, _last_act, _score) in eligible.into_iter().take(k) {
        // remove from active
        active.retain(|x| x != &mint);

        // mark inactive + enqueue
        if let Some(st) = coins.get_mut(&mint) {
            st.active = false;
            st.queued_since = now;
        }

        // Prevent immediate promote ping-pong
        shadow::shadow_for(shadow_map, mint.as_str(), now, rotate_shadow_secs);

        // Avoid duplicates in queue
        if !queued.contains(&mint) {
            queue.push_back(mint);
            rotated += 1;
        }
    }

    if rotated > 0 {
        counters.queue_rotated += rotated as u64;
        if debug_enabled() {
            eprintln!(
                "DBG rotate_least_active rotated={} active_now={} queue_now={}",
                rotated,
                active.len(),
                queue.len()
            );
        }
    }
}

/// Heartbeat snapshots for mints sitting in the queue/watch-band.
/// Goal: keep a lightweight FDV/tx history so momentum + staleness logic works,
/// without snapshotting the whole universe.
///
/// Recommended cadence:
/// - queue/watch-band: 45–90s (start with 60s)
///
/// Notes:
/// - Uses CoinState.last_snapshot_ts as a per-mint throttle.
/// - Writes is_active=false, is_call=false snapshots.
/// - Optionally writes events=0 (wire db.events_5m if you want).
pub fn snapshot_queue_heartbeat(
    coins: &mut HashMap<String, CoinState>,
    queue: &VecDeque<String>,
    market: &MarketCache,
    db: &mut Db,
    now_ts: i64,
    counters: &mut Counters,
) {
    let queue_interval_sec: i64 = 60;

    // cap how many we snapshot per tick to avoid burst writes
    let per_tick_cap: usize = 200;

    let mut wrote: usize = 0;

    for mint in queue.iter().take(per_tick_cap) {
        let Some(st) = coins.get(mint) else { continue };

        let last_snap: i64 = st.last_snapshot_ts;
        let due: bool = last_snap == 0 || (now_ts - last_snap) >= queue_interval_sec;
        if !due {
            continue;
        }

        let Some(ms) = market.map.get(mint) else {
            continue;
        };

        let fdv = ms.fdv;
        let tx5 = ms.tx_5m;
        let score = st.score;
        let first_seen = st.first_seen;
        let signers = st.unique_signers_5m as u64;

        // If we know absolutely nothing, skip
        if fdv.unwrap_or(0.0) <= 0.0 && tx5.unwrap_or(0) == 0 && signers == 0 {
            continue;
        }

        // ✅ pull the real 5m events count from DB
        let ev: usize = db.events_5m(now_ts, mint.as_str()).unwrap_or(0) as usize;
        // queued heartbeat snapshot: is_active=false, is_call=false
        let _ = db.insert_snapshot(
            now_ts,
            mint.as_str(),
            fdv,
            tx5,
            score,
            signers,
            ev, // ✅ instead of 0
            first_seen,
            false, // is_active
            false, // is_call
        );

        if let Some(stm) = coins.get_mut(mint) {
            stm.last_snapshot_ts = now_ts;
        }

        counters.snapshots_wrote += 1;
        wrote += 1;
    }

    if wrote > 0 && debug_enabled() {
        eprintln!("DBG snapshot_queue_heartbeat wrote={}", wrote);
    }
}

/// Promote from queue into active.
/// - enforces TTL
/// - revalidates score + shadow
/// - requires market presence (price-first model)
/// - clears queued_since on promotion
/// - avoids active duplicates
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

        let Some(st) = coins.get(&mint) else { continue };

        // TTL drop
        if st.queued_since > 0 && now.saturating_sub(st.queued_since) > QUEUE_TTL_SEC {
            counters.queue_dropped_ttl += 1;
            continue;
        }

        // Must be promotable
        if st.score < 20 || shadow::is_shadowed(shadow_map, mint.as_str(), now) {
            continue;
        }

        // Must have market data (price-first)
        if !market.map.contains_key(&mint) {
            continue;
        }

        // Avoid duplicates in active
        if active.iter().any(|x| x == &mint) {
            continue;
        }

        active.push(mint.clone());
        if let Some(stm) = coins.get_mut(&mint) {
            stm.active = true;
            stm.active_since = now;
            stm.queued_since = 0;
        }

        println!(
            "{}",
            crate::fmt::active_line(&mint, coins.get(&mint).map(|s| s.score).unwrap_or(0))
        );
    }
}

/// Remove mints that were shadowed after calls (safety cleanup).
pub fn remove_shadowed_after_calls(active: &mut Vec<String>, shadowed: &[String]) {
    if shadowed.is_empty() {
        return;
    }
    let set: HashSet<&str> = shadowed.iter().map(|s| s.as_str()).collect();
    active.retain(|x| !set.contains(x.as_str()));
}
