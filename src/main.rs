// src/main.rs

mod banner;
mod call_log;
mod config;
mod db;
mod fmt;
mod gambol_log;
mod governor;
mod helius;
mod io;
mod market;
mod printing;
mod pumpportal;
mod resolver;
mod scoring;
mod telegram;
mod time;
mod types;

use crate::config::load_config;
use crate::governor::Governor;
use crate::market::cache::MarketCache;
use crate::market::discovery::{merge_discovered, MarketDiscovery};
use crate::types::{CallRecord, CoinState};

use std::collections::{HashMap, HashSet, VecDeque};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::{interval, MissedTickBehavior};

use tokio_util::sync::CancellationToken;

fn debug_enabled() -> bool {
    std::env::var("DEBUG").ok().as_deref() == Some("1")
}

#[tokio::main]
async fn main() {
    let cfg = load_config();

    // ONE shutdown token for the whole process (clone + pass everywhere)
    let shutdown = CancellationToken::new();

    // Ctrl+C watcher: cancels the token immediately
    {
        let shutdown = shutdown.clone();
        tokio::spawn(async move {
            let _ = tokio::signal::ctrl_c().await;
            eprintln!("🛑 Ctrl+C received — shutting down now...");
            shutdown.cancel();
        });
    }

    if debug_enabled() {
        eprintln!("DBG cwd={}", std::env::current_dir().unwrap().display());
        eprintln!("DBG .env exists? {}", std::path::Path::new(".env").exists());
        eprintln!(
            "DBG raw env MIN_CALL_FDV_USD={:?} MAX_CALL_FDV_USD={:?}",
            std::env::var("MIN_CALL_FDV_USD").ok(),
            std::env::var("MAX_CALL_FDV_USD").ok(),
        );
    }

    crate::banner::initializing();

    // throttles / periodic timers
    let mut last_watchlist_promote_ts: i64 = 0;
    let mut last_outcomes_eval_ts: i64 = 0;

    eprintln!(
        "CFG call band: min_call_fdv_usd={} max_call_fdv_usd={}",
        cfg.min_call_fdv_usd, cfg.max_call_fdv_usd
    );

    // -------------------------
    // Bot state
    // -------------------------
    let mut coins: HashMap<String, CoinState> = HashMap::new();
    let mut active: Vec<String> = Vec::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    let mut calls: Vec<CallRecord> = Vec::new();

    let mut market = MarketCache::default();
    let mut discovered: VecDeque<String> = VecDeque::new();
    let mut discovery = MarketDiscovery::default();
    let mut shadow = scoring::shadow::ShadowMap::new();

    let (tg_tx, mut tg_rx) = tokio::sync::mpsc::channel::<String>(100);

    // -------------------------
    // DB (one canonical open)
    // -------------------------
    eprintln!("DB sqlite_path = {}", cfg.sqlite_path);
    let mut db = crate::db::Db::open(&cfg.sqlite_path).expect("db open failed");

    // -------------------------
    // PumpPortal stream
    // -------------------------
    let (pump_tx, mut pump_rx) = mpsc::channel::<String>(10_000);
    let cfg_pp = cfg.clone();

    // Governor (shared rate limits)
    let gov = std::sync::Arc::new(Governor::new(
        45.0, 90.0, // rpc_rps, rpc_burst
        9.0, 18.0, // das_rps, das_burst
        9.0, 18.0, // enh_rps, enh_burst
        4,    // inflight_limit (2..6 recommended)
    ));

    // PumpPortal task
    tokio::spawn({
        let gov_pp = gov.clone();
        async move {
            pumpportal::client::run(cfg_pp, pump_tx, gov_pp).await;
        }
    });

    println!(
        "{}🚀 Solana Meme Sniper started{} (Ctrl+C to stop)",
        fmt::NEON_GREEN,
        fmt::RESET
    );

    // =========================
    // MAIN LOOP (interval-driven)
    // =========================
    let mut tick = interval(Duration::from_millis(250));
    tick.set_missed_tick_behavior(MissedTickBehavior::Skip);

    // Heartbeat throttling (don't spam every tick)
    let heartbeat_every: i64 = 10; // seconds
    let mut last_heartbeat_ts: i64 = 0;

    'main: loop {
        tokio::select! {
            biased;

            // Ctrl+C watcher cancels `shutdown`
            _ = shutdown.cancelled() => {
                break 'main;
            }

            _ = tick.tick() => {
                if shutdown.is_cancelled() {
                    break 'main;
                }

                // ------------------------------------------------------------
                // 0) Time (once per tick)
                // ------------------------------------------------------------
                let now: u64 = crate::time::now();
                let now_ts: i64 = now as i64;

                // ------------------------------------------------------------
                // 1) Drain PumpPortal mints (fast path)
                // ------------------------------------------------------------
                drain_pump_mints(&mut pump_rx, &mut coins, &mut discovered, &shutdown);

                // ------------------------------------------------------------
                // 2) Periodic discovery (slow path)
                // ------------------------------------------------------------
                if !shutdown.is_cancelled() && discovery.should_run(&cfg) {
                    let new_mints = discovery.run(&cfg).await;

                    if shutdown.is_cancelled() {
                        break 'main;
                    }

                    let added = merge_discovered(&mut discovered, new_mints.clone(), 2_000);

                    if added > 0 && !shutdown.is_cancelled() {
                        println!(
                            "{}🕵️ discovery{} got={} added={} coins={}",
                            fmt::LIGHT_BLUE,
                            fmt::RESET,
                            new_mints.len(),
                            added,
                            coins.len()
                        );
                    }

                    for mint in new_mints {
                        coins.entry(mint).or_insert_with(CoinState::new);
                    }
                }

                // ------------------------------------------------------------
                // 3) Market polling (Dex) + pair_address propagation
                // ------------------------------------------------------------
                if !shutdown.is_cancelled() {
                    poll_market_and_propagate_pairs(
                        &cfg,
                        &mut market,
                        &mut coins,
                        &active,
                        &queue,
                        &shutdown,
                    ).await;

                    if shutdown.is_cancelled() {
                        break 'main;
                    }
                }

                // ------------------------------------------------------------
                // 4) Snapshot history warmup (for momentum gates)
                // ------------------------------------------------------------
                if !shutdown.is_cancelled() {
                    snapshot_candidates(
                        &mut db,
                        &coins,
                        &market,
                        &active,
                        &queue,
                        now_ts,
                        200,
                        &shutdown,
                    );
                }

                // ------------------------------------------------------------
                // 5) Watchlist promote (throttled)
                // ------------------------------------------------------------
                if !shutdown.is_cancelled() {
                    maybe_promote_watchlist(&mut db, now_ts, &mut last_watchlist_promote_ts);
                }

                // ------------------------------------------------------------
                // 6) Wallet-learning ingest (top wallets)
                // ------------------------------------------------------------
                if !shutdown.is_cancelled() {
                    let wallets = build_wallet_learning_list(&cfg, &mut db, 25);
                    if !wallets.is_empty() {
                        if let Err(e) = crate::helius::ingest::ingest_wallet_activity(
                            &cfg, &mut db, &mut coins, &wallets, gov.clone(), &shutdown
                        ).await {
                            if !shutdown.is_cancelled() {
                                eprintln!("{}DBG wallet-learning ingest ERR={:?}{}", fmt::RED, e, fmt::RESET);
                            }
                        }
                    }

                    if shutdown.is_cancelled() {
                        break 'main;
                    }
                }

                // ------------------------------------------------------------
                // 7) Pair-tracking ingest (active + queue)
                // ------------------------------------------------------------
                if !shutdown.is_cancelled() {
                    let tracked = build_tracked_pair_addresses(&coins, &active, &queue, 150);
                    if !tracked.is_empty() {
                        let discovered_mints = crate::scoring::onchain::fetch_onchain_events(
                            &cfg, &mut db, &mut coins, &tracked, gov.clone(), &shutdown
                        ).await;

                        if shutdown.is_cancelled() {
                            break 'main;
                        }

                        for m in discovered_mints {
                            coins.entry(m).or_insert_with(CoinState::new);
                        }
                    }
                }

                // ------------------------------------------------------------
                // 8) Per-coin ingest (active)
                // ------------------------------------------------------------
                if !shutdown.is_cancelled() && !active.is_empty() {
                    if let Err(e) = crate::helius::per_coin::ingest_pairs_for_active(
                        &cfg, &mut db, &mut coins, &active, gov.clone()
                    ).await {
                        if !shutdown.is_cancelled() {
                            eprintln!("{}DBG per_coin ingest ERR={:?}{}", fmt::RED, e, fmt::RESET);
                        }
                    }

                    if shutdown.is_cancelled() {
                        break 'main;
                    }
                }

                // ------------------------------------------------------------
                // 9) Wallet reputation update
                // ------------------------------------------------------------
                if !shutdown.is_cancelled() {
                    if let Err(e) = crate::scoring::wallet_rep::update_wallet_reputation(&mut db, now_ts) {
                        if !shutdown.is_cancelled() {
                            eprintln!("{}DBG wallet_rep ERR={:?}{}", fmt::RED, e, fmt::RESET);
                        }
                    }
                }

                // ------------------------------------------------------------
                // 10) Score & manage (active/queue/calls)
                // ------------------------------------------------------------
                if !shutdown.is_cancelled() {
                    crate::scoring::engine::score_and_manage(
                        &cfg,
                        &mut coins,
                        &mut active,
                        &mut queue,
                        &mut calls,
                        &market,
                        &mut shadow,
                        &mut db,
                        &tg_tx,
                    );

                    while let Ok(msg) = tg_rx.try_recv() {
                        let token = cfg.telegram_bot_token.clone();
                        let chat_id = cfg.telegram_chat_id.clone();
                        tokio::spawn(async move {
                            telegram::send_alert(&token, &chat_id, &msg).await;
                        });
                    }
                }

                // ------------------------------------------------------------
                // 11) Outcomes + perf (throttled)
                // ------------------------------------------------------------
                if !shutdown.is_cancelled() {
                    if let Err(e) = crate::printing::tick_outcomes_and_perf(
                        &mut db,
                        &market,
                        &mut last_outcomes_eval_ts,
                        now_ts,
                        60,
                    ) {
                        if !shutdown.is_cancelled() {
                            eprintln!("{}DBG tick_outcomes_and_perf ERR={:?}{}", fmt::RED, e, fmt::RESET);
                        }
                    }
                }

                // ------------------------------------------------------------
                // 12) Heartbeat (throttled)
                // ------------------------------------------------------------
                if !shutdown.is_cancelled() {
                    if last_heartbeat_ts == 0 || (now_ts - last_heartbeat_ts) >= heartbeat_every {
                        last_heartbeat_ts = now_ts;
                        println!(
                            "{}🫀 hb{} local={} coins={} active={} queue={} calls={} discovered={}",
                            fmt::ORANGE,
                            fmt::RESET,
                            chrono::Local::now().format("%-I:%M:%S %p"),
                            coins.len(),
                            active.len(),
                            queue.len(),
                            calls.len(),
                            discovered.len(),
                        );
                    }
                }

                // Optional extra sleep (interval already paces ticks)
                if cfg.main_loop_sleep > 0 && !shutdown.is_cancelled() {
                    tokio::time::sleep(Duration::from_secs(cfg.main_loop_sleep)).await;
                }
            }
        }
    }
    crate::banner::goodbye();
}

// ============================================================
// Helpers (keep main loop readable)
// ============================================================

fn drain_pump_mints(
    pump_rx: &mut mpsc::Receiver<String>,
    coins: &mut HashMap<String, CoinState>,
    discovered: &mut VecDeque<String>,
    shutdown: &CancellationToken,
) {
    if shutdown.is_cancelled() {
        return;
    }

    let mut pump_added = 0usize;

    while let Ok(mint) = pump_rx.try_recv() {
        if shutdown.is_cancelled() {
            return;
        }

        if !coins.contains_key(&mint) {
            coins.entry(mint.clone()).or_insert_with(CoinState::new);
            discovered.push_back(mint);
            pump_added += 1;

            while discovered.len() > 20_000 {
                discovered.pop_front();
            }
        }
    }

    if pump_added > 0 && !shutdown.is_cancelled() {
        println!(
            "{}🟣 pumpportal{} added={} coins={}",
            fmt::NEON_PINK,
            fmt::RESET,
            pump_added,
            coins.len()
        );
    }
}

async fn poll_market_and_propagate_pairs(
    cfg: &crate::config::Config,
    market: &mut MarketCache,
    coins: &mut HashMap<String, CoinState>,
    active: &[String],
    queue: &VecDeque<String>,
    shutdown: &CancellationToken,
) {
    if shutdown.is_cancelled() {
        return;
    }

    // Priority = active + some queue
    let mut priority: Vec<String> = Vec::new();
    priority.extend(active.iter().cloned());
    priority.extend(queue.iter().take(200).cloned());

    // mint_list = priority first, then rest (capped), deduped
    let mut seen: HashSet<String> = HashSet::new();
    let mut mint_list: Vec<String> = Vec::new();

    for m in &priority {
        if seen.insert(m.clone()) {
            mint_list.push(m.clone());
        }
    }

    let mut added_rest = 0usize;
    for m in coins.keys() {
        if shutdown.is_cancelled() {
            return;
        }
        if seen.insert(m.clone()) {
            mint_list.push(m.clone());
            added_rest += 1;
            if added_rest >= 2_000 {
                break;
            }
        }
    }

    if shutdown.is_cancelled() {
        return;
    }
    market.poll(cfg, &priority, &mint_list).await;

    if shutdown.is_cancelled() {
        return;
    }

    // propagate pair_address -> CoinState
    let mut propagated = 0usize;
    for (mint, st) in coins.iter_mut() {
        if shutdown.is_cancelled() {
            return;
        }
        if st.pair_address.is_none() {
            if let Some(ms) = market.map.get(mint) {
                if let Some(pa) = ms.pair_address.as_ref() {
                    if !pa.trim().is_empty() {
                        st.pair_address = Some(pa.clone());
                        propagated += 1;
                    }
                }
            }
        }
    }

    if propagated > 0 && debug_enabled() && !shutdown.is_cancelled() {
        eprintln!("DBG pair_address propagated={}", propagated);
    }
}

fn snapshot_candidates(
    db: &mut crate::db::Db,
    coins: &HashMap<String, CoinState>,
    market: &MarketCache,
    active: &[String],
    queue: &VecDeque<String>,
    now_ts: i64,
    max_queue: usize,
    shutdown: &CancellationToken,
) {
    if shutdown.is_cancelled() {
        return;
    }

    let active_set: HashSet<&str> = active.iter().map(|s| s.as_str()).collect();

    // active + top of queue (dedup)
    let mut mints: Vec<String> = Vec::new();
    let mut seen: HashSet<String> = HashSet::new();

    for m in active {
        if seen.insert(m.clone()) {
            mints.push(m.clone());
        }
    }
    for m in queue.iter().take(max_queue) {
        if seen.insert(m.clone()) {
            mints.push(m.clone());
        }
    }

    let mut wrote = 0usize;

    for mint in mints {
        if shutdown.is_cancelled() {
            return;
        }

        let ms = market.map.get(&mint);

        let fdv = ms.and_then(|x| x.fdv);
        let tx5 = ms.and_then(|x| x.tx_5m);

        let score = coins.get(&mint).map(|s| s.score).unwrap_or(0);
        let signers = coins
            .get(&mint)
            .map(|s| s.unique_signers_5m as u64)
            .unwrap_or(0);
        let first_seen = coins.get(&mint).map(|s| s.first_seen).unwrap_or(0);

        // If we know absolutely nothing, skip
        if fdv.unwrap_or(0.0) <= 0.0 && tx5.unwrap_or(0) == 0 && signers == 0 {
            continue;
        }

        let ev = db.events_5m(now_ts, mint.as_str()).unwrap_or(0) as usize;
        let is_active = active_set.contains(mint.as_str());

        let _ = db.insert_snapshot(
            now_ts,
            mint.as_str(),
            fdv,
            tx5,
            score,
            signers,
            ev,
            first_seen,
            is_active,
            false,
        );

        wrote += 1;
    }

    if wrote > 0 && debug_enabled() && !shutdown.is_cancelled() {
        eprintln!("DBG snapshots wrote={}", wrote);
    }
}

fn build_tracked_pair_addresses(
    coins: &HashMap<String, CoinState>,
    active: &[String],
    queue: &VecDeque<String>,
    queue_cap: usize,
) -> Vec<String> {
    let mut tracked: Vec<String> = Vec::new();

    for m in active {
        if let Some(pa) = coins.get(m).and_then(|s| s.pair_address.clone()) {
            tracked.push(pa);
        }
    }

    for m in queue.iter().take(queue_cap) {
        if let Some(pa) = coins.get(m).and_then(|s| s.pair_address.clone()) {
            tracked.push(pa);
        }
    }

    tracked.sort();
    tracked.dedup();
    tracked
}

fn maybe_promote_watchlist(db: &mut crate::db::Db, now_ts: i64, last_ts: &mut i64) {
    if *last_ts != 0 && (now_ts - *last_ts) < 60 {
        return;
    }

    let promote_n = 5;
    let min_score = 50;
    let max_watchlist = 10;

    match db.promote_watchlist_from_scored_wallets(now_ts, promote_n, min_score, max_watchlist) {
        Ok((added, pruned)) => {
            if added > 0 || pruned > 0 {
                eprintln!(
                    "DBG watchlist promote: added={} pruned={} (min_score={} keep={})",
                    added, pruned, min_score, max_watchlist
                );
            }
        }
        Err(e) => eprintln!("DBG watchlist promote ERR={:?}", e),
    }

    *last_ts = now_ts;
}

fn build_wallet_learning_list(
    cfg: &crate::config::Config,
    db: &mut crate::db::Db,
    limit: usize,
) -> Vec<String> {
    let mut wallets = db.get_watchlist_wallets(limit);
    if wallets.is_empty() {
        wallets = cfg
            .helius_wallets
            .split(',')
            .map(|s| s.trim())
            .filter(|s| !s.is_empty())
            .map(|s| s.to_string())
            .collect();
    }
    wallets
}
