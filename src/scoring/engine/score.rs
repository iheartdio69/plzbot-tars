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
    _active: &Vec<String>, // currently unused; keep for future, avoid warning
) {
    for (mint, st) in coins.iter_mut() {
        counters.considered += 1;

        // ------------------------------------------------------------
        // 0) Age filter (ignore stale coins, e.g. > cfg.discovery_max_age_secs)
        // ------------------------------------------------------------
        let age_sec = now.saturating_sub(st.first_seen);
        if age_sec > cfg.discovery_max_age_secs {
            st.score = -999;
            st.wallet_delta = 0;
            st.active = false;
            st.low_score_streak = 0;
            st.skip_call_for_conc = false;
            continue;
        }

        // ------------------------------------------------------------
        // 1) Market snapshot required to score FDV/liquidity/tx_5m
        // ------------------------------------------------------------
        let Some(ms) = market.map.get(mint) else {
            // No market = don't score it yet. Keep neutral.
            st.score = 0;
            st.wallet_delta = 0;
            st.low_score_streak = 0;
            st.skip_call_for_conc = false;
            continue;
        };

        let fdv: f64 = ms.fdv.unwrap_or(0.0);
        let liq: f64 = ms.liq.unwrap_or(0.0);
        st.tx_5m = ms.tx_5m.unwrap_or(0) as usize;

        // Volume spike detection
        let spike_interval: u64 = 60;
        if st.prev_tx_ts == 0 || now.saturating_sub(st.prev_tx_ts) >= spike_interval {
            st.prev_tx_5m = st.tx_5m;
            st.prev_tx_ts = now;
        }
        let volume_spike: bool = st.prev_tx_5m >= 20
            && st.tx_5m >= st.prev_tx_5m * 2;

        // ------------------------------------------------------------
        // 2) HARD FDV CAP at watch layer
        // ------------------------------------------------------------
        if fdv > cfg.max_watch_fdv_usd {
            st.score = -999;
            st.wallet_delta = 0;
            st.active = false;
            st.low_score_streak = 0;
            st.skip_call_for_conc = false;
            continue;
        }

        // Keep last 20m of events
        let keep_since = now.saturating_sub(1200);
        st.events.retain(|e| e.ts >= keep_since);

        // ------------------------------------------------------------
        // 2.5) Signers (CRITICAL): never clobber window-derived signers
        //
        // st.unique_signers_5m is ideally computed from your window code
        // (distinct wallets in last 300s). DB can lag or count differently,
        // so treat DB as a floor, not a replacement.
        // ------------------------------------------------------------
        // Compute signers from in-memory events (zero DB queries)
        let cutoff_5m = now.saturating_sub(300);
        let mem_signers: usize = {
            let mut seen = std::collections::HashSet::new();
            for e in st.events.iter() {
                if e.ts >= cutoff_5m {
                    seen.insert(e.wallet.as_str());
                }
            }
            seen.len()
        };
        let db_signers: usize = db.signers_5m(now_ts, mint.as_str()).unwrap_or(0) as usize;

        st.unique_signers_5m = mem_signers.max(db_signers);

        // Reset conc flag each tick (gating may set it later)
        st.skip_call_for_conc = false;

        // ------------------------------------------------------------
        // 3) Quality scan last 5m (wallet scores)
        // ------------------------------------------------------------
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

        // Cache wallet_delta ONCE per tick (call loop reads it)
        let early_fdv = fdv < 25_000.0;
        st.wallet_delta = if early_fdv
            && quality_hits < 2
            && quality_sum < 80
            && (st.unique_signers_5m as u64) < 18
        {
            -40
        } else {
            0
        };

        // ------------------------------------------------------------
        // 4) Score calc
        // ------------------------------------------------------------
        let mut score: i32 = 0;

        // Watch band (min_watch..max_watch) — meaningful because of hard-cap above.
        if fdv >= cfg.min_watch_fdv_usd && fdv <= cfg.max_watch_fdv_usd {
            score += 25;
        } else if fdv > 0.0 && fdv < cfg.min_watch_fdv_usd {
            score += 5;
        }

        if liq >= cfg.min_liq_usd {
            score += 15;
        }

        // FDV change in last 5m
        if let Some(pct) = db.fdv_change_pct(mint.as_str(), now_ts, 300) {
            if pct >= 0.30 {
                score += 15;
            } else if pct >= 0.20 {
                score += 10;
            } else if pct >= 0.10 {
                score += 5;
            }
        }

        // FDV delta last 30s
        let fdv_delta_30s = db
            .fdv_delta_recent(mint.as_str(), now_ts, 30)
            .unwrap_or(0.0);

        if fdv_delta_30s >= 50_000.0 {
            score += 25;
        } else if fdv_delta_30s >= 25_000.0 {
            score += 15;
        } else if fdv_delta_30s >= 10_000.0 {
            score += 10;
        }

        // ------------------------------------------------------------
        // 4.x) Activity / distribution / events (raw additive version)
        // ------------------------------------------------------------
        let tx5: usize = st.tx_5m;
        let signers: usize = st.unique_signers_5m;

        let cutoff_5m = now.saturating_sub(300);
        let events_5m: usize = st
            .events
            .iter()
            .rev()
            .take_while(|e| e.ts >= cutoff_5m)
            .count();

        score += tx5.min(200) as i32;
        score += (signers.min(40) * 2) as i32;
        score += events_5m.min(50) as i32;

        // Wallet quality bonus
        if quality_hits >= 2 {
            score += 10;
        }
        if quality_hits >= 5 {
            score += 10;
        }
        if quality_sum >= 250 {
            score += 10;
        }

        // Whales last 5m
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

        // ------------------------------------------------------------
        // WHALE ENTRY BONUS: high-score watchlist wallet entered this mint
        // ------------------------------------------------------------
        let whale_threshold: i64 = 500;
        let whale_lookback: i64 = 300; // last 5 minutes

        let top_wallet_score: i64 = db
            .top_wallets_for_mint_window(
                mint.as_str(),
                now_ts - whale_lookback,
                now_ts,
                5,
            )
            .unwrap_or_default()
            .iter()
            .filter_map(|(w, _)| db.wallet_score(w).ok())
            .max()
            .unwrap_or(0);

        if top_wallet_score >= whale_threshold {
            score += 300;
            st.whale_entry = true;
            st.whale_entry_score = top_wallet_score;
        } else {
            st.whale_entry = false;
            st.whale_entry_score = 0;
        }

        // Volume spike bonus
        if volume_spike {
            score += 300;
            st.is_volume_spike = true;
        } else {
            st.is_volume_spike = false;
        }

        // Shadow penalty
        if shadow::is_shadowed(shadow_map, mint.as_str(), now) {
            score -= 50;
        }

        st.score = score;

        // ------------------------------------------------------------
        // 5) Demotion streak
        // ------------------------------------------------------------
        if st.score < cfg.score_demote {
            st.low_score_streak = st.low_score_streak.saturating_add(1);
        } else {
            st.low_score_streak = 0;
        }

        // ------------------------------------------------------------
        // 6) Active rotation TTL
        // ------------------------------------------------------------
        if st.active && (now - st.active_since) > ACTIVE_TTL_SEC {
            st.active = false;
        }

        // No-progress deactivation
        let last_event_ts = st.events.last().map(|e| e.ts).unwrap_or(0);
        if st.active && (now - last_event_ts) > NO_PROGRESS_TTL_SEC {
            st.active = false;
        }

        // SNAPSHOT DISABLED (speed test)
    }
}
