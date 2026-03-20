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
    cfg: &Config,
    coins: &mut HashMap<String, CoinState>,
    active: &mut Vec<String>,
    queue: &mut VecDeque<String>,
    shadow_map: &mut shadow::ShadowMap,
    now: u64,
    counters: &mut Counters,
) {
    let _ = shadow_map;
    let target = cfg.max_active_coins.max(10);

    // Always fill empty slots first — best scored first
    while active.len() < target {
        let best_pos = queue
            .iter()
            .enumerate()
            .max_by_key(|(_, m)| coins.get(*m).map(|s| s.score).unwrap_or(0))
            .map(|(i, _)| i);

        match best_pos {
            Some(pos) => {
                let mint = queue.remove(pos).unwrap();
                if let Some(st) = coins.get_mut(&mint) {
                    st.active = true;
                    st.active_since = now;
                }
                active.push(mint);
            }
            None => break,
        }
    }

    // Merit swap — replace worst with best if significantly better
    if !queue.is_empty() && active.len() >= target {
        let worst_score = active
            .iter()
            .map(|m| coins.get(m).map(|s| s.score).unwrap_or(0))
            .min()
            .unwrap_or(0);

        let best_queued = queue
            .iter()
            .enumerate()
            .max_by_key(|(_, m)| coins.get(*m).map(|s| s.score).unwrap_or(0));

        if let Some((pos, best_mint)) = best_queued {
            let best_score = coins.get(best_mint).map(|s| s.score).unwrap_or(0);
            if best_score > worst_score + 50 {
                let new_mint = queue.remove(pos).unwrap();
                if let Some(worst_mint) = active
                    .iter()
                    .min_by_key(|m| coins.get(*m).map(|s| s.score).unwrap_or(0))
                    .cloned()
                {
                    active.retain(|m| m != &worst_mint);
                    if let Some(st) = coins.get_mut(&worst_mint) {
                        st.active = false;
                    }
                    queue.push_front(worst_mint);
                }
                if let Some(st) = coins.get_mut(&new_mint) {
                    st.active = true;
                    st.active_since = now;
                }
                active.push(new_mint);
                counters.queue_rotated += 1;
            }
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
/// - sorted: whale/spike/recovery emergency boost + score
pub fn promote_from_queue(
    cfg: &Config,
    coins: &mut HashMap<String, CoinState>,
    active: &mut Vec<String>,
    queue: &mut VecDeque<String>,
    market: &MarketCache,
    shadow_map: &mut shadow::ShadowMap,
    db: &mut Db,
    now: u64,
    now_ts: i64,
    counters: &mut Counters,
) {
    if active.len() >= cfg.max_active_coins {
        return;
    }

    let slots = cfg.max_active_coins - active.len();

    let mut heat_scores: Vec<(String, i64)> = queue
        .iter()
        .map(|mint| {
            let score = coins.get(mint).map(|s| s.score as i64).unwrap_or(0);

            let st = coins.get(mint);
            let emergency_boost: i64 = if st.map(|s| s.whale_entry).unwrap_or(false) {
                10_000
            } else if st.map(|s| s.is_volume_spike).unwrap_or(false) {
                5_000
            } else if st.map(|s| s.is_recovery).unwrap_or(false) {
                2_000
            } else {
                0
            };

            (mint.clone(), emergency_boost + score)
        })
        .collect();

    heat_scores.sort_by(|a, b| b.1.cmp(&a.1));

    let mut promoted = 0usize;

    for (mint, heat) in heat_scores.iter() {
        if promoted >= slots || active.len() >= cfg.max_active_coins {
            break;
        }

        let Some(st) = coins.get(mint) else {
            queue.retain(|m| m != mint);
            continue;
        };

        // TTL drop
        if st.queued_since > 0 && now.saturating_sub(st.queued_since) > QUEUE_TTL_SEC {
            queue.retain(|m| m != mint);
            counters.queue_dropped_ttl += 1;
            continue;
        }

        // Must be promotable
        if st.score < 20 || shadow::is_shadowed(shadow_map, mint.as_str(), now) {
            continue;
        }

        // Must have market data (price-first)
        if !market.map.contains_key(mint) {
            continue;
        }

        // Avoid duplicates in active
        if active.iter().any(|x| x == mint) {
            queue.retain(|m| m != mint);
            continue;
        }

        active.push(mint.clone());
        queue.retain(|m| m != mint);

        if let Some(stm) = coins.get_mut(mint) {
            stm.active = true;
            stm.active_since = now;
            stm.queued_since = 0;
        }

        println!(
            "{}",
            crate::fmt::active_line(mint, coins.get(mint).map(|s| s.score).unwrap_or(0))
        );


        promoted += 1;
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
