use super::Counters;
use crate::config::Config;
use crate::market::cache::MarketCache;
use crate::scoring::shadow;
use crate::types::CoinState;
use std::collections::{HashMap, HashSet, VecDeque};

fn is_bonk_like_mint(mint: &str) -> bool {
    mint.to_ascii_lowercase().contains("bonk")
}

pub fn fill_queue(
    cfg: &Config,
    coins: &mut HashMap<String, CoinState>,
    active: &Vec<String>,
    queue: &mut VecDeque<String>,
    market: &MarketCache,
    shadow_map: &shadow::ShadowMap,
    counters: &mut Counters,
) {
    let now = crate::time::now();

    let active_set: HashSet<&str> = active.iter().map(|s| s.as_str()).collect();
    let queue_set: HashSet<&str> = queue.iter().map(|s| s.as_str()).collect();

    let max_queue_len: usize = 500;
    if queue.len() >= max_queue_len {
        return;
    }

    // Collect (mint, score) so we can sort
    let mut candidates: Vec<(String, i32)> = Vec::new();

    for (mint, st) in coins.iter() {
        counters.considered += 1;

        // ------------------------------------------------------------
        // 0) Hard filters (no market needed)
        // ------------------------------------------------------------
        if active_set.contains(mint.as_str()) || queue_set.contains(mint.as_str()) {
            continue;
        }

        if shadow::is_shadowed(shadow_map, mint.as_str(), now) {
            continue;
        }

        if is_bonk_like_mint(mint) {
            continue;
        }

        let age_sec = now.saturating_sub(st.first_seen);
        if age_sec > cfg.discovery_max_age_secs {
            continue;
        }

        // ------------------------------------------------------------
        // 1) Require market data + FDV watch band
        // ------------------------------------------------------------
        let Some(ms) = market.map.get(mint) else {
            continue;
        };

        let fdv = ms.fdv.unwrap_or(0.0);
        if fdv < cfg.min_watch_fdv_usd || fdv > cfg.max_watch_fdv_usd {
            continue;
        }

        let tx5 = ms.tx_5m.unwrap_or(0);
        let mem_signers = st.unique_signers_5m as u64;

        // Only flag extreme bots — high tx with almost zero wallets
        if tx5 >= 500 && mem_signers <= 2 {
            continue; // obvious bot
        }
        if tx5 >= 200 && mem_signers == 0 {
            continue; // zero wallets is suspicious
        }

        // ------------------------------------------------------------
        // 2) Score threshold
        // ------------------------------------------------------------
        if st.score >= cfg.queue_score_min {
            candidates.push((mint.clone(), st.score));
        }
    }

    // ------------------------------------------------------------
    // 3) Best-first ordering
    // ------------------------------------------------------------
    candidates.sort_by(|a, b| {
        let a_whale = coins.get(&a.0).map(|s| s.whale_entry).unwrap_or(false);
        let b_whale = coins.get(&b.0).map(|s| s.whale_entry).unwrap_or(false);
        let a_spike = coins.get(&a.0).map(|s| s.is_volume_spike).unwrap_or(false);
        let b_spike = coins.get(&b.0).map(|s| s.is_volume_spike).unwrap_or(false);
        let a_recovery = coins.get(&a.0).map(|s| s.is_recovery).unwrap_or(false);
        let b_recovery = coins.get(&b.0).map(|s| s.is_recovery).unwrap_or(false);
        b_whale.cmp(&a_whale)
            .then(b_spike.cmp(&a_spike))
            .then(b_recovery.cmp(&a_recovery))
            .then(b.1.cmp(&a.1))
    });

    for (mint, _) in candidates {
        if queue.len() >= max_queue_len {
            break;
        }

        if let Some(st) = coins.get_mut(&mint) {
            st.queued_since = now;
        }

        queue.push_back(mint);
    }

    // ------------------------------------------------------------
    // DEBUG: queue health (once per tick)
    // ------------------------------------------------------------
    if std::env::var("DEBUG").ok().as_deref() == Some("1") {
        eprintln!(
            "DBG QUEUE filled={} active={} coins={}",
            queue.len(),
            active.len(),
            coins.len()
        );
    }
}
