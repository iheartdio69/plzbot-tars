mod active;
mod calls;
mod constants;
mod counters;
mod queue;
mod score;
mod summary;

pub use active::{promote_from_queue, remove_shadowed_after_calls, update_active_list};
pub use calls::process_calls;
pub use constants::*;
pub use counters::Counters;
pub use queue::fill_queue;
pub use score::score_all_coins;
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
    tg_tx: &tokio::sync::mpsc::Sender<String>,
) {
    let now = crate::time::now();
    let now_ts = now as i64;

    let debug: bool = std::env::var("DEBUG").ok().as_deref() == Some("1");
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

    // 2) clean active list (drop demoted/inactive; shadow briefly)
    update_active_list(cfg, coins, active, shadow_map, now);

    // 3) rotate least-active out (continuous cycling)
    // NOTE: rotate_least_active signature includes shadow_map.
    active::rotate_least_active(cfg, coins, active, queue, shadow_map, now, &mut counters);

    // 4) fill queue from scored coins
    fill_queue(cfg, coins, active, queue, market, shadow_map, &mut counters);

    // 4.5) queue/watch-band heartbeat snapshots (lightweight FDV baselines)
    active::snapshot_queue_heartbeat(coins, queue, market, db, now_ts, &mut counters);

    // 5) promote from queue into active
    promote_from_queue(
        cfg,
        coins,
        active,
        queue,
        market,
        shadow_map,
        now,
        &mut counters,
    );

    // 6) process calls (returns mints shadowed-after-call)
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
        tg_tx,
    );

    // 7) remove shadowed mints from active
    remove_shadowed_after_calls(active, &shadowed_after_call);

    // 8) per-tick debug (ONE line per tick; do NOT put inside per-mint loops)
    if debug {
        eprintln!(
            "DBG TICK: coins={} active={} queue={} snapshots_written={}",
            coins.len(),
            active.len(),
            queue.len(),
            counters.snapshots_wrote
        );
    }

    // 9) summary
    print_summary(&counters, active.len(), queue.len());
}
