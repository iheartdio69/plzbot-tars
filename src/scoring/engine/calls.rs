use super::Counters;
use crate::config::Config;
use crate::market::cache::MarketCache;
use crate::scoring::shadow;
use crate::types::{CallRecord, CoinState};
use chrono::{Local, TimeZone};
use std::collections::{HashMap, HashSet};

fn debug_enabled() -> bool {
    std::env::var("DEBUG").ok().as_deref() == Some("1")
}

fn is_bonk_like_mint(mint: &str) -> bool {
    mint.to_ascii_lowercase().contains("bonk")
}

/// Returns an approximate FDV from ~5m ago using existing snapshots.
/// We pick a value in [now-360, now-240] and take the peak in that window.
fn fdv_approx_300s_ago(db: &mut crate::db::Db, now_ts: i64, mint: &str) -> Option<f64> {
    let start = now_ts - 360;
    let end = now_ts - 240;

    db.peak_fdv_for_mint_window(mint, start, end)
        .ok()
        .flatten()
        .filter(|v| *v > 100.0)
}

/// FDV at ~60s ago (windowed)
fn fdv_approx_60s_ago(db: &mut crate::db::Db, now_ts: i64, mint: &str) -> Option<f64> {
    let target: i64 = now_ts - 60;
    let min_ts: i64 = target - 30;
    db.snapshot_fdv_at_or_before(mint, target, min_ts)
        .ok()
        .flatten()
        .filter(|v| *v > 100.0)
}

fn skip_and_maybe_demote(
    reason: &str,
    bucket: &str,
    mint: &str,
    dbg_skips_left: &mut i32,
    counters: &mut Counters,
    coins: &mut HashMap<String, CoinState>,
    remove_from_active: &mut HashSet<String>,
    shadow_map: &mut shadow::ShadowMap,
    db: &mut crate::db::Db,
    fdv_usd: Option<f64>,
    cfg: &Config,
    now: u64,
    now_ts: i64,
    demote_shadow_secs: u64,
) {
    // Record skip (best-effort)
    let _ = db.insert_skip_debug(now_ts, mint, reason, bucket, fdv_usd);

    // Debug print (limited)
    if debug_enabled() && *dbg_skips_left > 0 {
        *dbg_skips_left -= 1;
        eprintln!(
            "DBG SKIP reason={} bucket={} mint={}",
            reason,
            bucket,
            crate::fmt::mint(mint)
        );
    }

    // Counters
    match bucket {
        "fdv" => counters.skip_fdv += 1,
        "conc" => counters.skip_conc += 1,
        "wallet" => counters.skip_wallet += 1,
        "cooldown" => {
            counters.skip_cooldown += 1;
            counters.skipped_cooldown += 1;
        }
        "signer" => counters.skip_signer += 1,
        "events" => counters.skip_events += 1,
        "revive" => counters.skip_other += 1,
        _ => counters.skip_other += 1,
    }
    counters.skipped_threshold += 1;

    // Never demote on cooldown (intentionally waiting)
    if bucket == "cooldown" {
        return;
    }

    // Already removed this tick
    if remove_from_active.contains(mint) {
        return;
    }

    let Some(st) = coins.get_mut(mint) else { return; };

    // Only ACTIVE coins are eligible for demotion
    if !st.active {
        return;
    }

    // Hard safety: concentration risk demotes immediately
    let hard_demote_now = bucket == "conc" || reason == "CONC_RISK";
    if hard_demote_now {
        st.active = false;
        st.demote_streak = 0;
        st.last_demote_ts = now_ts;

        remove_from_active.insert(mint.to_string());
        shadow::shadow_for(shadow_map, mint, now, demote_shadow_secs);

        if debug_enabled() {
            eprintln!(
                "DBG DEMOTE_ACTIVE mint={} reason={} bucket={} streak=HARD",
                crate::fmt::mint(mint),
                reason,
                bucket
            );
        }
        return;
    }

    // Soft demote: build a streak (FDV failures count heavier)
    if bucket == "fdv" {
        st.demote_streak = st.demote_streak.saturating_add(2);
    } else {
        st.demote_streak = st.demote_streak.saturating_add(1);
    }

    // Threshold check
    let threshold: u32 = cfg.demote_streak.max(1);
    if st.demote_streak >= threshold {
        st.active = false;
        st.demote_streak = 0;
        st.last_demote_ts = now_ts;

        remove_from_active.insert(mint.to_string());
        shadow::shadow_for(shadow_map, mint, now, demote_shadow_secs);

        if debug_enabled() {
            eprintln!(
                "DBG DEMOTE_ACTIVE mint={} reason={} bucket={} streak_reached={}",
                crate::fmt::mint(mint),
                reason,
                bucket,
                threshold
            );
        }
    }
}

pub fn process_calls(
    cfg: &Config,
    coins: &mut HashMap<String, CoinState>,
    active: &mut Vec<String>,
    calls: &mut Vec<CallRecord>,
    market: &MarketCache,
    shadow_map: &mut shadow::ShadowMap,
    db: &mut crate::db::Db,
    now: u64,
    now_ts: i64,
    counters: &mut Counters,
    tg_tx: &tokio::sync::mpsc::Sender<String>,
) -> Vec<String> {
    // ----------------------------
    // Tunables
    // ----------------------------
    let cooldown_secs: i64 = 15 * 60;

    // Startup warmup (per-mint, based on age)
    let momentum_warmup_sec: u64 = 120;

    // Events-conviction gate (pre-call)
    let ev_min_for_conviction: i64 = 60;
    let eps_min: f64 = 2.2;
    let fdv_follow_pct_min: f64 = 0.03;

    // Call FDV band (HARD)
    let call_fdv_min: f64 = cfg.min_call_fdv_usd;

    // GAMBOL lane
    let gambol_fdv_floor: f64 = 20_000.0;
    let gambol_min_ev: u64 = 2;
    let gambol_min_tx5: u64 = 120;
    let gambol_max_signers: u64 = 3;

    // Anti-spoof / deadzone
    let dead_tx_min: u64 = 30;
    let spoof_tx_min: u64 = 120;
    let spoof_signers_max: u64 = 2;

    // Refined-only gates
    let wallet_delta_min: i32 = -10;
    let refined_min_signer_strength: u64 = 12;

    // Age gates
    let soft_max_age_sec: u64 = 2 * 60 * 60;
    let hard_max_age_sec: u64 = 2 * 24 * 60 * 60;

    // Late-entry
    let late_prev_window_sec: i64 = 30 * 60;
    let late_exclude_tail_sec: i64 = 120;
    let late_ratio: f64 = 1.35;

    // Concentration
    let conc_window_sec: i64 = 300;
    let conc_min_edges_sample: i64 = 80;
    let conc_top1_thresh: f64 = 0.35;
    let conc_top5_thresh: f64 = 0.75;

    // Wallet-quality
    let wallet_quality_min_ge10: i64 = 2;
    let wallet_veto_score: i64 = -50;
    let wallet_veto_top1_pct: f64 = 0.18;

    // Newborn quality
    let min_signers_new: u64 = 20;
    let min_events_new: u64 = 35;
    let min_eps_new: f64 = 1.8;
    let top1_pct_bad: f64 = 0.22;
    let top1_score_bad: i64 = -20;

    // Runner lane
    let runner_fdv_min: f64 = 700_000.0;

    let debug: bool = debug_enabled();
    let mut dbg_skips_left: i32 = 30;

    let mut shadowed_after_call: Vec<String> = Vec::new();
    let active_snapshot = active.clone();

    // Active rotation helpers
    let start_active_len = active.len();
    let mut remove_from_active: HashSet<String> = HashSet::new();
    let demote_shadow_secs: u64 = 60;

    for mint in active_snapshot.iter() {
        counters.considered += 1;

        // Warmup flag for momentum gate (per-mint, based on age)
        let first_seen: u64 = coins.get(mint).map(|s| s.first_seen).unwrap_or(0);
        let age_sec: u64 = now.saturating_sub(first_seen);
        let in_momentum_warmup: bool = age_sec < momentum_warmup_sec;

        // 0) Quick excludes
        if is_bonk_like_mint(mint) {
            skip_and_maybe_demote(
                "BONK_LIKE",
                "other",
                mint,
                &mut dbg_skips_left,
                counters,
                coins,
                &mut remove_from_active,
                shadow_map,
                db,
                None,
                cfg,
                now,
                now_ts,
                demote_shadow_secs,
            );
            continue;
        }

        let ms = match market.map.get(mint) {
            Some(v) => v,
            None => {
                skip_and_maybe_demote(
                    "NO_MARKET_SAMPLE",
                    "other",
                    mint,
                    &mut dbg_skips_left,
                    counters,
                    coins,
                    &mut remove_from_active,
                    shadow_map,
                    db,
                    None,
                    cfg,
                    now,
                    now_ts,
                    demote_shadow_secs,
                );
                continue;
            }
        };

        // 1) Pull signals
        let fdv: f64 = ms.fdv.unwrap_or(0.0);
        let tx5: u64 = ms.tx_5m.unwrap_or(0);

        let mem_signers_5m: u64 = coins
            .get(mint)
            .map(|s| s.unique_signers_5m as u64)
            .unwrap_or(0);

        let ev: u64 = db.events_5m(now_ts, mint.as_str()).unwrap_or(0) as u64;
        let db_uniq_sigs_5m: u64 = db.uniq_sigs_5m(now_ts, mint.as_str()).unwrap_or(0);

        let signer_strength: u64 = mem_signers_5m.max(db_uniq_sigs_5m);
        let signers: u64 = signer_strength;

        let wallet_delta: i32 = coins.get(mint).map(|s| s.wallet_delta).unwrap_or(0);

        // Snapshot heartbeat (don’t swallow errors)
        {
            let interval_sec: i64 = 10;

            let last_snap: i64 = coins
                .get(mint.as_str())
                .map(|s| s.last_snapshot_ts)
                .unwrap_or(0);

            let due: bool = last_snap == 0 || (now_ts - last_snap) >= interval_sec;

            if due {
                match db.insert_snapshot(
                    now_ts,
                    mint.as_str(),
                    ms.fdv,
                    ms.tx_5m,
                    coins.get(mint.as_str()).map(|s| s.score).unwrap_or(0),
                    signers,
                    ev as usize,
                    first_seen,
                    true,
                    false,
                ) {
                    Ok(_) => {
                        if let Some(st) = coins.get_mut(mint.as_str()) {
                            st.last_snapshot_ts = now_ts;
                        }
                        counters.snapshots_wrote += 1;
                    }
                    Err(e) => {
                        eprintln!(
                            "DBG insert_snapshot ERR mint={} err={:?}",
                            crate::fmt::mint(mint),
                            e
                        );
                    }
                }
            }
        }

        // 2) Gate: FDV floor
        if fdv < call_fdv_min {
            skip_and_maybe_demote(
                "FDV_TOO_LOW",
                "fdv",
                mint,
                &mut dbg_skips_left,
                counters,
                coins,
                &mut remove_from_active,
                shadow_map,
                db,
                Some(fdv),
                cfg,
                now,
                now_ts,
                demote_shadow_secs,
            );
            continue;
        }

        // 3) Decide lane: refined vs gambol
        let gambol_ok: bool = fdv >= gambol_fdv_floor
            && ev >= gambol_min_ev
            && tx5 >= gambol_min_tx5
            && signers <= gambol_max_signers;

        // 4) Late-entry filter (refined only)
        if !gambol_ok {
            let peak_prev_30m: f64 = db
                .peak_fdv_for_mint_window(
                    mint.as_str(),
                    now_ts - late_prev_window_sec,
                    now_ts - late_exclude_tail_sec,
                )
                .unwrap_or(None)
                .unwrap_or(0.0);

            if peak_prev_30m > 0.0 && fdv > 0.0 && peak_prev_30m >= fdv * late_ratio {
                skip_and_maybe_demote(
                    "LATE_ENTRY",
                    "other",
                    mint,
                    &mut dbg_skips_left,
                    counters,
                    coins,
                    &mut remove_from_active,
                    shadow_map,
                    db,
                    Some(fdv),
                    cfg,
                    now,
                    now_ts,
                    demote_shadow_secs,
                );
                continue;
            }
        }

        // 5) Spam guards (hard)
        if signers == 0 && ev == 0 {
            skip_and_maybe_demote(
                "DEADZONE_NO_SIGNERS_NO_EVENTS",
                "other",
                mint,
                &mut dbg_skips_left,
                counters,
                coins,
                &mut remove_from_active,
                shadow_map,
                db,
                Some(fdv),
                cfg,
                now,
                now_ts,
                demote_shadow_secs,
            );
            continue;
        }

        if tx5 >= spoof_tx_min && signers <= spoof_signers_max {
            skip_and_maybe_demote(
                "SPOOF_BUSY_LOW_SIGNERS",
                "other",
                mint,
                &mut dbg_skips_left,
                counters,
                coins,
                &mut remove_from_active,
                shadow_map,
                db,
                Some(fdv),
                cfg,
                now,
                now_ts,
                demote_shadow_secs,
            );
            continue;
        }

        if tx5 < dead_tx_min && signers < 3 && ev < 3 {
            skip_and_maybe_demote(
                "LOW_SIGNAL",
                "other",
                mint,
                &mut dbg_skips_left,
                counters,
                coins,
                &mut remove_from_active,
                shadow_map,
                db,
                Some(fdv),
                cfg,
                now,
                now_ts,
                demote_shadow_secs,
            );
            continue;
        }

        // 6) Refined onchain signal (refined only)
        if !gambol_ok {
            let refined_onchain_ok: bool =
                signers > 0 || ev > 0 || signer_strength >= refined_min_signer_strength;

            if !refined_onchain_ok {
                skip_and_maybe_demote(
                    "REFINED_ONCHAIN_WEAK",
                    "other",
                    mint,
                    &mut dbg_skips_left,
                    counters,
                    coins,
                    &mut remove_from_active,
                    shadow_map,
                    db,
                    Some(fdv),
                    cfg,
                    now,
                    now_ts,
                    demote_shadow_secs,
                );
                continue;
            }
        }

        // 7) Refined momentum (refined only)
        if !gambol_ok {
            let required_chg_5m: f64 = if fdv < 30_000.0 { 0.10 } else { 0.07 };
            let baseline_fdv_opt: Option<f64> = fdv_approx_300s_ago(db, now_ts, mint.as_str());

            match baseline_fdv_opt {
                Some(fdv_5m) => {
                    let chg_5m: f64 = (fdv / fdv_5m) - 1.0;
                    if chg_5m < required_chg_5m {
                        skip_and_maybe_demote(
                            "REFINED_NO_ACCEL_5M",
                            "other",
                            mint,
                            &mut dbg_skips_left,
                            counters,
                            coins,
                            &mut remove_from_active,
                            shadow_map,
                            db,
                            Some(fdv),
                            cfg,
                            now,
                            now_ts,
                            demote_shadow_secs,
                        );
                        continue;
                    }
                }
                None => {
                    if !in_momentum_warmup {
                        skip_and_maybe_demote(
                            "NO_SNAPSHOT_HISTORY_FOR_MOMENTUM",
                            "other",
                            mint,
                            &mut dbg_skips_left,
                            counters,
                            coins,
                            &mut remove_from_active,
                            shadow_map,
                            db,
                            Some(fdv),
                            cfg,
                            now,
                            now_ts,
                            demote_shadow_secs,
                        );
                        continue;
                    }
                }
            }
        }

        // 8) Wallet delta (refined only)
        if !gambol_ok && wallet_delta < wallet_delta_min {
            skip_and_maybe_demote(
                "WALLET_DELTA_TOO_NEG",
                "wallet",
                mint,
                &mut dbg_skips_left,
                counters,
                coins,
                &mut remove_from_active,
                shadow_map,
                db,
                Some(fdv),
                cfg,
                now,
                now_ts,
                demote_shadow_secs,
            );
            continue;
        }

        // 9) Age rules
        let revival_by_age_ok: bool = age_sec > soft_max_age_sec && signer_strength >= 25;

        if age_sec > hard_max_age_sec {
            skip_and_maybe_demote(
                "AGE_TOO_OLD_HARD",
                "other",
                mint,
                &mut dbg_skips_left,
                counters,
                coins,
                &mut remove_from_active,
                shadow_map,
                db,
                Some(fdv),
                cfg,
                now,
                now_ts,
                demote_shadow_secs,
            );
            continue;
        }

        if age_sec > soft_max_age_sec && !revival_by_age_ok {
            skip_and_maybe_demote(
                "AGE_TOO_OLD_SOFT",
                "other",
                mint,
                &mut dbg_skips_left,
                counters,
                coins,
                &mut remove_from_active,
                shadow_map,
                db,
                Some(fdv),
                cfg,
                now,
                now_ts,
                demote_shadow_secs,
            );
            continue;
        }

        if !gambol_ok && signer_strength < 10 {
            skip_and_maybe_demote(
                "REFINED_TOO_FEW_REAL_SIGNERS",
                "signer",
                mint,
                &mut dbg_skips_left,
                counters,
                coins,
                &mut remove_from_active,
                shadow_map,
                db,
                Some(fdv),
                cfg,
                now,
                now_ts,
                demote_shadow_secs,
            );
            continue;
        }

        // 10) Cooldown
        if let Ok(Some(last_ts)) = db.last_call_ts_for_mint(mint.as_str()) {
            if now_ts - last_ts < cooldown_secs {
                skip_and_maybe_demote(
                    "COOLDOWN",
                    "cooldown",
                    mint,
                    &mut dbg_skips_left,
                    counters,
                    coins,
                    &mut remove_from_active,
                    shadow_map,
                    db,
                    Some(fdv),
                    cfg,
                    now,
                    now_ts,
                    demote_shadow_secs,
                );
                continue;
            }
        }

        // ------------------------------------------------------------
        // PRE-CALL: top wallets + concentration + wallet quality
        // ------------------------------------------------------------
        let call_ts: i64 = now_ts;
        let start_ts: i64 = call_ts - conc_window_sec;
        let end_ts: i64 = call_ts;

        let top25: Vec<(String, i64)> = db
            .top_wallets_for_mint_window(mint.as_str(), start_ts, end_ts, 25)
            .unwrap_or_default();

        let mut conc_risk: bool = false;
        let mut conc_total_edges: i64 = 0;
        let mut conc_top1_edges: i64 = 0;
        let mut conc_top5_edges: i64 = 0;
        let mut conc_top1_pct: f64 = 0.0;
        let mut conc_top5_pct: f64 = 0.0;
        let mut conc_top1_wallet: String = String::new();

        if !top25.is_empty() {
            conc_total_edges = db
                .total_edges_for_mint_window(mint.as_str(), start_ts, end_ts)
                .unwrap_or(0);

            if conc_total_edges > 0 {
                conc_top1_edges = top25.get(0).map(|(_, e)| *e).unwrap_or(0);
                conc_top5_edges = top25.iter().take(5).map(|(_, e)| *e).sum();

                conc_top1_pct = conc_top1_edges as f64 / conc_total_edges as f64;
                conc_top5_pct = conc_top5_edges as f64 / conc_total_edges as f64;

                conc_top1_wallet = top25.get(0).map(|(w, _)| w.clone()).unwrap_or_default();

                let enough_sample: bool = conc_total_edges >= conc_min_edges_sample;
                conc_risk = enough_sample
                    && (conc_top1_pct >= conc_top1_thresh || conc_top5_pct >= conc_top5_thresh);
            }
        }

        if conc_risk {
            if let Some(st) = coins.get_mut(mint.as_str()) {
                st.skip_call_for_conc = true;
            }

            skip_and_maybe_demote(
                "CONC_RISK",
                "conc",
                mint,
                &mut dbg_skips_left,
                counters,
                coins,
                &mut remove_from_active,
                shadow_map,
                db,
                Some(fdv),
                cfg,
                now,
                now_ts,
                demote_shadow_secs,
            );
            continue;
        }

        // Wallet quality (refined only)
        let mut ge10_cnt: i64 = 0;
        if !gambol_ok {
            for (w, _) in top25.iter().take(10) {
                if db.wallet_score(w).unwrap_or(0) >= 10 {
                    ge10_cnt += 1;
                }
            }

            if let Some((w0, _)) = top25.get(0) {
                let veto_bad_min: bool = db.wallet_score(w0).unwrap_or(0) <= wallet_veto_score;
                if veto_bad_min {
                    skip_and_maybe_demote(
                        "WALLET_VETO_BAD_MIN",
                        "wallet",
                        mint,
                        &mut dbg_skips_left,
                        counters,
                        coins,
                        &mut remove_from_active,
                        shadow_map,
                        db,
                        Some(fdv),
                        cfg,
                        now,
                        now_ts,
                        demote_shadow_secs,
                    );
                    continue;
                }
            }

            if ge10_cnt < wallet_quality_min_ge10 {
                skip_and_maybe_demote(
                    "WALLET_QUALITY_TOO_LOW",
                    "wallet",
                    mint,
                    &mut dbg_skips_left,
                    counters,
                    coins,
                    &mut remove_from_active,
                    shadow_map,
                    db,
                    Some(fdv),
                    cfg,
                    now,
                    now_ts,
                    demote_shadow_secs,
                );
                continue;
            }
        } else {
            for (w, _) in top25.iter().take(10) {
                if db.wallet_score(w).unwrap_or(0) >= 10 {
                    ge10_cnt += 1;
                }
            }
        }

        // Final top1 veto (all lanes)
        let top1_wallet_score: i64 = if !conc_top1_wallet.is_empty() {
            db.wallet_score(&conc_top1_wallet).unwrap_or(0)
        } else {
            0
        };

        let veto_bad_top1: bool =
            top1_wallet_score <= wallet_veto_score && conc_top1_pct >= wallet_veto_top1_pct;

        if veto_bad_top1 {
            skip_and_maybe_demote(
                "TOP1_VETO",
                "wallet",
                mint,
                &mut dbg_skips_left,
                counters,
                coins,
                &mut remove_from_active,
                shadow_map,
                db,
                Some(fdv),
                cfg,
                now,
                now_ts,
                demote_shadow_secs,
            );
            continue;
        }

        // ------------------------------------------------------------
        // Revival detection + staleness (pre-call)
        // ------------------------------------------------------------
        let revive_gap_secs: i64 = 20 * 60;
        let revive_lookback_secs: i64 = 6 * 60 * 60;

        let max_gap_secs: i64 = db
            .mint_max_gap_secs_recent(mint.as_str(), now_ts - revive_lookback_secs, now_ts)
            .ok()
            .flatten()
            .unwrap_or(0);

        let revival_ok: bool = max_gap_secs >= revive_gap_secs;

        let stale_secs: i64 = 300;
        let revive_grace_secs: i64 = 90;

        let mut last_snapshot_ts: i64 = db
            .mint_last_seen_ts(mint.as_str())
            .ok()
            .flatten()
            .unwrap_or(0);

        if last_snapshot_ts > now_ts {
            last_snapshot_ts = now_ts;
        }

        if last_snapshot_ts > 0 && (now_ts - last_snapshot_ts) > stale_secs {
            skip_and_maybe_demote(
                "STALE_MINT",
                "revive",
                mint,
                &mut dbg_skips_left,
                counters,
                coins,
                &mut remove_from_active,
                shadow_map,
                db,
                Some(fdv),
                cfg,
                now,
                now_ts,
                demote_shadow_secs,
            );
            continue;
        }

        if revival_ok && last_snapshot_ts > 0 && (now_ts - last_snapshot_ts) > revive_grace_secs {
            skip_and_maybe_demote(
                "REVIVE_NOT_ACTIVE_NOW",
                "revive",
                mint,
                &mut dbg_skips_left,
                counters,
                coins,
                &mut remove_from_active,
                shadow_map,
                db,
                Some(fdv),
                cfg,
                now,
                now_ts,
                demote_shadow_secs,
            );
            continue;
        }

        // ------------------------------------------------------------
        // Events conviction + FDV follow-through (pre-call)
        // ------------------------------------------------------------
        if (ev as i64) >= ev_min_for_conviction {
            let eps: f64 = if signers > 0 { ev as f64 / signers as f64 } else { 0.0 };

            let base_60 = fdv_approx_60s_ago(db, now_ts, mint.as_str());
            let fdv_follow_ok: bool = if let Some(b) = base_60 {
                ((fdv - b) / b) >= fdv_follow_pct_min
            } else {
                in_momentum_warmup
            };

            if eps < eps_min || !fdv_follow_ok {
                skip_and_maybe_demote(
                    "EVENTS_NO_CONVICTION",
                    "events",
                    mint,
                    &mut dbg_skips_left,
                    counters,
                    coins,
                    &mut remove_from_active,
                    shadow_map,
                    db,
                    Some(fdv),
                    cfg,
                    now,
                    now_ts,
                    demote_shadow_secs,
                );
                continue;
            }
        }

        // ------------------------------------------------------------
        // NEWBORN quality (pre-call, refined only)
        // ------------------------------------------------------------
        if !gambol_ok && age_sec <= 180 {
            let eps: f64 = if signers > 0 { ev as f64 / signers as f64 } else { 0.0 };

            if signers < min_signers_new || ev < min_events_new || eps < min_eps_new {
                skip_and_maybe_demote(
                    "NEWBORN_QUALITY",
                    "signer",
                    mint,
                    &mut dbg_skips_left,
                    counters,
                    coins,
                    &mut remove_from_active,
                    shadow_map,
                    db,
                    Some(fdv),
                    cfg,
                    now,
                    now_ts,
                    demote_shadow_secs,
                );
                continue;
            }

            if conc_top1_pct >= top1_pct_bad && top1_wallet_score <= top1_score_bad {
                skip_and_maybe_demote(
                    "NEWBORN_TOP1_BAD",
                    "conc",
                    mint,
                    &mut dbg_skips_left,
                    counters,
                    coins,
                    &mut remove_from_active,
                    shadow_map,
                    db,
                    Some(fdv),
                    cfg,
                    now,
                    now_ts,
                    demote_shadow_secs,
                );
                continue;
            }
        }

        // ------------------------------------------------------------
        // RUNNER lane: must be moving NOW (uses ~60s baseline)
        // ------------------------------------------------------------
        if fdv >= runner_fdv_min {
            let base_60 = fdv_approx_60s_ago(db, now_ts, mint.as_str());
            let chg_60_ok: bool = base_60
                .map(|b| b > 0.0 && ((fdv / b) - 1.0) >= 0.06)
                .unwrap_or(false);

            let delta_60s: f64 = db.fdv_delta_recent(mint.as_str(), now_ts, 60).unwrap_or(0.0);
            let delta_ok: bool = delta_60s >= 75_000.0;

            let activity_ok: bool = tx5 >= 250 && signer_strength >= 20;

            if !(activity_ok && (chg_60_ok || delta_ok)) {
                skip_and_maybe_demote(
                    "RUNNER_NOT_MOVING",
                    "other",
                    mint,
                    &mut dbg_skips_left,
                    counters,
                    coins,
                    &mut remove_from_active,
                    shadow_map,
                    db,
                    Some(fdv),
                    cfg,
                    now,
                    now_ts,
                    demote_shadow_secs,
                );
                continue;
            }
        }

        // ------------------------------------------------------------
        // ✅ PRE-CALL: move your old “post-call” vetoes here
        // ------------------------------------------------------------
        let (tot_edges, _t1e, top1_pct, _t5e, top5_pct) = db
            .mint_concentration_5m(now_ts, mint)
            .unwrap_or((0, 0, 0.0, 0, 0.0));

        let (_uniq_src, _edges_total, edges_per_wallet) =
            db.mint_edge_stats_5m(now_ts, mint).unwrap_or((0, 0, 0.0));

        let sol_flow_5m = db.mint_sol_flow_5m(now_ts, mint).unwrap_or(0.0);

        if top1_pct >= 0.30 {
            skip_and_maybe_demote(
                "CONC_TOP1",
                "conc",
                mint,
                &mut dbg_skips_left,
                counters,
                coins,
                &mut remove_from_active,
                shadow_map,
                db,
                Some(fdv),
                cfg,
                now,
                now_ts,
                demote_shadow_secs,
            );
            continue;
        }

        if top5_pct >= 0.45 {
            skip_and_maybe_demote(
                "CONC_TOP5",
                "conc",
                mint,
                &mut dbg_skips_left,
                counters,
                coins,
                &mut remove_from_active,
                shadow_map,
                db,
                Some(fdv),
                cfg,
                now,
                now_ts,
                demote_shadow_secs,
            );
            continue;
        }

        let avg_sol_per_edge = if tot_edges > 0 {
            sol_flow_5m / (tot_edges as f64)
        } else {
            0.0
        };

        if edges_per_wallet >= 3.5 && avg_sol_per_edge < 0.25 {
            skip_and_maybe_demote(
                "SPAM_EDGES",
                "other",
                mint,
                &mut dbg_skips_left,
                counters,
                coins,
                &mut remove_from_active,
                shadow_map,
                db,
                Some(fdv),
                cfg,
                now,
                now_ts,
                demote_shadow_secs,
            );
            continue;
        }

        // ------------------------------------------------------------
        // 14) ACCEPT CALL
        // ------------------------------------------------------------
        let base_score: i32 = coins.get(mint).map(|s| s.score).unwrap_or(0);
        let effective_score: i32 = base_score.saturating_add(wallet_delta);

        calls.push(CallRecord {
            mint: mint.clone(),
            ts: now,
            score: effective_score,
        });
        counters.called += 1;

        // -----------------
        // Tags + Color (ALWAYS tagged, 1 primary color)
        // -----------------
        let newborn: bool = age_sec <= 180;

        // Tune these whenever (this is just sane defaults)
        let runner_fdv_min: f64 = 700_000.0;
        let mid_fdv_min: f64 = 120_000.0;

        // ----- primary lane tag (ALWAYS ONE) -----
        let lane_tag: &str = if gambol_ok {
            "GAMBOL"
        } else if revival_ok {
            "REVIVE"
        } else if fdv >= runner_fdv_min {
            "RUNNER"
        } else if newborn {
            "NEWBORN"
        } else if fdv >= mid_fdv_min {
            "MID"
        } else {
            "SMALL"
        };

        // ----- tags list -----
        let mut tags: Vec<&str> = Vec::new();
        tags.push(lane_tag);

        // wallet delta “flavor” tags
        if wallet_delta > 0 {
            tags.push("WPLUS");
        } else if wallet_delta < 0 {
            tags.push("WNEG");
        }

        let tag_owned: String = tags.join("|");
        let tag_opt: Option<&str> = Some(tag_owned.as_str());

        // -----------------
        // Print (color by lane)
        // -----------------
        let line = crate::fmt::call_line_tagged(
            mint.as_str(),
            fdv,
            effective_score,
            tx5,
            signers as usize,
            ev as usize,
            tag_owned.as_str(), // "" if no tags
        );
        let colored_line = match lane_tag {
            "GAMBOL" => crate::fmt::red(&line),
            "REVIVE" => crate::fmt::yellow(&line),
            "RUNNER" => crate::fmt::green(&line),
            "NEWBORN" => crate::fmt::cyan(&line),
            _ => line.clone(),
        };

        println!("{}", colored_line);
        println!(
            "   🏷 {}   🕒 local={}",
            tag_owned,
            chrono::Local::now().format("%-I:%M:%S %p")
        );

        let tg_msg = format!(
            "🎯 <b>INTERFECTOR</b>\nMint: <code>{}</code>\nLane: {}\nFDV: ${:.0}\nScore: {} | TX: {} | Signers: {}\n🔗 https://axiom.trade/t/{}",
            mint, lane_tag, fdv, effective_score, tx5, signers, mint
        );
        let _ = tg_tx.try_send(tg_msg);

        // Insert call
        if let Err(e) = db.insert_call(
            call_ts,
            mint.as_str(),
            fdv,
            effective_score,
            tx5,
            signers,
            ev as usize,
            tag_opt,
        ) {
            eprintln!(
                "DBG insert_call ERR mint={} err={:?}",
                crate::fmt::mint(mint),
                e
            );
            continue;
        }

        // Snap top wallets at call time (re-use top25)
        if !top25.is_empty() {
            if let Err(e) = db.insert_call_top_wallets(mint.as_str(), call_ts, &top25) {
                eprintln!(
                    "DBG insert_call_top_wallets ERR mint={} err={:?}",
                    crate::fmt::mint(mint),
                    e
                );
            }
        }

        let top1_is_whale: bool = if !conc_top1_wallet.is_empty() {
            db.watchlist_has_wallet(&conc_top1_wallet).unwrap_or(false)
        } else {
            false
        };

        if let Err(e) = db.insert_call_debug(
            call_ts,
            mint.as_str(),
            fdv,
            effective_score,
            tx5,
            signers,
            ev as usize,
            signer_strength,
            age_sec,
            wallet_delta,
            conc_total_edges,
            conc_top1_edges,
            conc_top5_edges,
            conc_top1_pct,
            conc_top5_pct,
            conc_top1_wallet.as_str(),
            top1_wallet_score,
            top1_is_whale,
            ge10_cnt,
            veto_bad_top1,
            max_gap_secs,
            revival_ok,
            gambol_ok,
            conc_risk,
        ) {
            eprintln!(
                "DBG insert_call_debug ERR mint={} err={:?}",
                crate::fmt::mint(mint),
                e
            );
        }

        // Snapshot mint at call time
        let _ = db.insert_snapshot(
            call_ts,
            mint.as_str(),
            Some(fdv),
            ms.tx_5m,
            effective_score,
            signers,
            ev as usize,
            first_seen,
            true,
            true,
        );

        // Log file
        let local_time = Local
            .timestamp_opt(call_ts, 0)
            .single()
            .map(|dt| dt.format("%Y-%m-%d %H:%M:%S").to_string())
            .unwrap_or_else(|| "invalid-ts".to_string());

        let log = format!(
            "{},{},{},{:.0},{},{},{}",
            local_time, call_ts, mint, fdv, effective_score, tx5, signers
        );
        crate::call_log::append_call_line(&log);

        shadow::shadow_for(shadow_map, mint.as_str(), now, 180);
        shadowed_after_call.push(mint.clone());

        if let Some(st) = coins.get_mut(mint) {
            st.active = false;
            st.demote_streak = 0;
            st.last_demote_ts = 0;
        }

        // remove from active vec this tick
        remove_from_active.insert(mint.clone());
    } // end for mint

    // Remove demoted/called mints from active ONCE per tick
    if !remove_from_active.is_empty() {
        active.retain(|m| !remove_from_active.contains(m));
    }

    let removed = start_active_len.saturating_sub(active.len());
    eprintln!(
        "DBG ACTIVE_ROTATION start={} removed={} end={}",
        start_active_len,
        removed,
        active.len()
    );

    shadowed_after_call
}
