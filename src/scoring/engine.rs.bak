use crate::config::Config;
use crate::market::cache::MarketCache;
use crate::scoring::shadow;
use crate::types::{CallRecord, CoinState, WhaleTier};
use std::collections::{HashMap, VecDeque};

const ACTIVE_TTL_SEC: u64 = 600;      // 10 min max active
const NO_PROGRESS_TTL_SEC: u64 = 120; // 2 min no new events
const QUEUE_TTL_SEC: u64 = 900;       // 15 min max in queue

pub fn score_and_manage(
    cfg: &Config,
    coins: &mut HashMap<String, CoinState>,
    active: &mut Vec<String>,
    queue: &mut VecDeque<String>,
    calls: &mut Vec<CallRecord>,
    market: &MarketCache,
    shadow_map: &mut shadow::ShadowMap,
    db: &mut crate::db::Db,
) {
    let now = crate::time::now();
    let now_ts = now as i64;

    let mut considered = 0usize;
    let mut called = 0usize;

    // skip counters
    let mut skipped_cooldown = 0usize;
    let mut skipped_threshold = 0usize;

    let mut skip_fdv = 0usize;
    let mut skip_conc = 0usize;
    let mut skip_wallet = 0usize;
    let mut skip_signer = 0usize;
    let mut skip_cooldown = 0usize;
    let mut skip_other = 0usize;
    let mut queue_dropped_ttl = 0usize;

    // ------------------------------------------------------------
    // 1) SCORE EACH COIN + ACTIVE ROTATION + CACHE wallet_delta
    // ------------------------------------------------------------
    for (mint, st) in coins.iter_mut() {
        considered += 1;

        let Some(ms) = market.map.get(mint) else {
            st.score = 0;
            st.wallet_delta = 0;
            continue;
        };

        let fdv = ms.fdv.unwrap_or(0.0);
        let liq = ms.liq.unwrap_or(0.0);
        let tx_5m = ms.tx_5m.unwrap_or(0) as usize;
        st.tx_5m = tx_5m;

        // keep last 20m of events
        let keep_since = now.saturating_sub(1200);
        st.events.retain(|e| e.ts >= keep_since);

        // signers for this mint
        st.unique_signers_5m = db.signers_5m(now_ts, mint.as_str()).unwrap_or(0) as usize;

        // quality scan on last 5 minutes (cap 200 events)
        let mut quality_hits: i64 = 0;
        let mut quality_sum: i64 = 0;
        let cutoff = now.saturating_sub(300);
        let mut sampled = 0usize;

        for e in st.events.iter().rev() {
            if e.ts < cutoff {
                break;
            }
            sampled += 1;
            if sampled > 200 {
                break;
            }
            let sc = db.wallet_score(&e.wallet).unwrap_or(0);
            if sc >= 10 {
                quality_hits += 1;
                quality_sum += sc.min(500);
            }
        }

        // cache wallet_delta ONCE per tick
        let early_fdv = fdv < 25_000.0;
        st.wallet_delta =
            if early_fdv && quality_hits < 2 && quality_sum < 80 && st.unique_signers_5m < 18 {
                -40
            } else {
                0
            };

        // base score
        let mut score: i32 = 0;

        if fdv >= cfg.min_watch_fdv_usd && fdv <= cfg.max_watch_fdv_usd {
            score += 15;
        }
        if liq >= cfg.min_liq_usd {
            score += 15;
        }

        if let Some(pct) = db.fdv_change_pct(mint.as_str(), now_ts, 300) {
            if pct >= 0.30 {
                score += 15;
            } else if pct >= 0.20 {
                score += 10;
            } else if pct >= 0.10 {
                score += 5;
            }
        }

        let fdv_delta_30s = db.fdv_delta_recent(mint.as_str(), now_ts, 30).unwrap_or(0.0);
        if fdv_delta_30s >= 50_000.0 {
            score += 25;
        } else if fdv_delta_30s >= 25_000.0 {
            score += 15;
        } else if fdv_delta_30s >= 10_000.0 {
            score += 10;
        }

        if st.tx_5m >= 5 {
            score += 15;
        }

        if !st.events.is_empty() {
            match st.unique_signers_5m {
                0 => score -= 5,
                1..=2 => score += 5,
                3..=5 => score += 10,
                _ => score += 20,
            }
        }

        if quality_hits >= 2 {
            score += 10;
        }
        if quality_hits >= 5 {
            score += 10;
        }
        if quality_sum >= 250 {
            score += 10;
        }

        // whales last 5 minutes
        let whale_cutoff = now.saturating_sub(300);
        let mut beluga: i32 = 0;
        let mut blue: i32 = 0;
        for e in st.events.iter().rev() {
            if e.ts < whale_cutoff {
                break;
            }
            match e.tier {
                WhaleTier::Beluga => beluga += 1,
                WhaleTier::Blue => blue += 1,
                WhaleTier::None => {}
            }
        }
        score += beluga * 5;
        score += blue * 10;

        if shadow::is_shadowed(shadow_map, mint, now) {
            score -= 50;
        }

        st.score = score;

        // demotion streak
        if st.score < cfg.score_demote {
            st.low_score_streak = st.low_score_streak.saturating_add(1);
        } else {
            st.low_score_streak = 0;
        }

        // optional: keep this field but not needed for decision logic
        st.skip_call_for_conc = false;

        // ACTIVE rotation TTL
        if st.active && (now - st.active_since) > ACTIVE_TTL_SEC {
            st.active = false;
            eprintln!(
                "DBG deactivate mint={} reason=ttl active_since={}",
                crate::fmt::mint(mint),
                st.active_since
            );
        }

        // no-progress deactivation
        let last_event_ts = st.events.last().map(|e| e.ts).unwrap_or(0);
        if st.active && (now - last_event_ts) > NO_PROGRESS_TTL_SEC {
            st.active = false;
            eprintln!(
                "DBG deactivate mint={} reason=no_progress last_event={}s ago",
                crate::fmt::mint(mint),
                now - last_event_ts
            );
        }

        // snapshot
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

    // ------------------------------------------------------------
    // 2) RECOMPUTE ACTIVE LIST: DROP DEMOTED + INACTIVE
    // ------------------------------------------------------------
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

    // ------------------------------------------------------------
    // 3) FILL QUEUE WITH CANDIDATES + set queued_since safely
    // ------------------------------------------------------------
    let mut to_enqueue: Vec<String> = Vec::new();

    for (mint, st) in coins.iter() {
        if st.score >= 20
            && !active.contains(mint)
            && !queue.contains(mint)
            && !shadow::is_shadowed(shadow_map, mint, now)
        {
            to_enqueue.push(mint.clone());
        }
    }

    for mint in to_enqueue {
        if let Some(st) = coins.get_mut(&mint) {
            st.queued_since = now;
        }
        queue.push_back(mint);
    }

    // ------------------------------------------------------------
    // 4) PROMOTE FROM QUEUE INTO ACTIVE (TTL + revalidation)
    // ------------------------------------------------------------
    while active.len() < cfg.max_active_coins {
        let Some(mint) = queue.pop_front() else { break };

        // TTL check
        if let Some(st) = coins.get(&mint) {
            if st.queued_since > 0 && (now - st.queued_since) > QUEUE_TTL_SEC {
                queue_dropped_ttl += 1;
                continue;
            }
        }

        // revalidate
        if let Some(st) = coins.get(&mint) {
            if st.score < 20 || shadow::is_shadowed(shadow_map, mint.as_str(), now) {
                continue;
            }
        } else {
            continue;
        }

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

    // ------------------------------------------------------------
    // 5) CREATE CALLS FROM ACTIVE COINS
    // ------------------------------------------------------------
    let active_snapshot: Vec<String> = active.clone();
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

        // FDV bounds
        if fdv < 5_000.0 || fdv > 1_000_000.0 {
            skip_fdv += 1;
            skipped_threshold += 1;
            continue;
        }

        let gambol_ok = (signers == 0) && (tx5 >= 200);

        // signer/tx spam guards
        if signers == 0 && tx5 < 50 && signer_strength < 8 {
            skip_signer += 1;
            skipped_threshold += 1;
            continue;
        }

        let tx_spam = tx5 >= 200 && signers == 0 && ev == 0;
        let low_signal = tx5 >= 80 && signers == 0 && ev == 0;
        if tx_spam || low_signal {
            skip_other += 1;
            skipped_threshold += 1;
            continue;
        }

        // wallet gate (use cached wallet_delta; no rescanning)
        let wallet_delta = coins.get(mint).map(|s| s.wallet_delta).unwrap_or(0);
        if wallet_delta < 0 {
            skip_wallet += 1;
            skipped_threshold += 1;
            // IMPORTANT: this was intended as a hard gate in your earlier logic
            continue;
        }

        // onchain signal gate
        let onchain_ok = (signers > 0) || (ev > 0) || (signer_strength >= 12);
        if !onchain_ok && !gambol_ok {
            skip_other += 1;
            skipped_threshold += 1;
            continue;
        }

        // age gate
        let max_age_sec: u64 = 2 * 60 * 60;
        let revival_ok: bool = signer_strength >= 25;
        if age_sec > max_age_sec && !revival_ok {
            skip_other += 1;
            skipped_threshold += 1;
            continue;
        }

        if !gambol_ok && signer_strength < 10 {
            skip_signer += 1;
            skipped_threshold += 1;
            continue;
        }

        // cooldown
        let cooldown_secs: i64 = 15 * 60;
        if let Ok(Some(last_ts)) = db.last_call_ts_for_mint(mint.as_str()) {
            if now_ts - last_ts < cooldown_secs {
                skipped_cooldown += 1;
                skip_cooldown += 1;
                continue;
            }
        }

        // concentration PRE-CALL gate (no state flag required)
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
            skip_conc += 1;
            skipped_threshold += 1;
            continue;
        }

        // accept call
        let base_score = coins.get(mint).map(|s| s.score).unwrap_or(0);
        let effective_score = base_score + wallet_delta; // wallet_delta is 0 here due to gate, but safe

        calls.push(CallRecord {
            mint: mint.clone(),
            ts: now,
            score: effective_score,
        });
        called += 1;

        let term_line = if gambol_ok {
            crate::fmt::red(&crate::fmt::call_line(
                mint.as_str(),
                fdv,
                effective_score,
                tx5,
                signers as usize,
                ev,
            ))
        } else {
            crate::fmt::call_line(mint.as_str(), fdv, effective_score, tx5, signers as usize, ev)
        };
        println!("{}", term_line);

        let tag = if gambol_ok { Some("GAMBOL") } else { None };

        if let Err(e) = db.insert_call(call_ts, mint, fdv, effective_score, tx5, signers, ev, tag) {
            eprintln!("DBG db insert_call failed mint={} err={:?}", crate::fmt::mint(mint), e);
        } else {
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
    }

    // ------------------------------------------------------------
    // 6) POST-CALL: DROP SHADOWED FROM ACTIVE
    // ------------------------------------------------------------
    for mint in shadowed_after_call.iter() {
        active.retain(|m| m != mint);
    }

    // ------------------------------------------------------------
    // 7) SUMMARY
    // ------------------------------------------------------------
    println!(
        "{}🧮 scoring{} considered={} called={} skips: fdv={} conc={} wallet={} cooldown={} signer={} other={} queue_dropped={} active={} queue={}",
        crate::fmt::YELLOW,
        crate::fmt::RESET,
        considered,
        called,
        skip_fdv,
        skip_conc,
        skip_wallet,
        skip_cooldown,
        skip_signer,
        skip_other,
        queue_dropped_ttl,
        active.len(),
        queue.len()
    );

    // keep these if you still want visibility
    let _ = skipped_cooldown;
    let _ = skipped_threshold;
}