// src/main.rs

mod banner;
mod call_log;
mod config;
mod db;
mod enrichment;
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
mod tars;
mod telegram;
mod time;
mod types;

use crate::config::load_config;
use crate::governor::Governor;
use crate::market::cache::MarketCache;
use crate::market::discovery::{merge_discovered, MarketDiscovery};
use crate::types::{CallRecord, CoinState};

use std::collections::{HashMap, HashSet, VecDeque};
use std::sync::{atomic::{AtomicI64, Ordering}, Arc};
use std::time::Duration;

use tokio::sync::mpsc;
use tokio::time::{interval, MissedTickBehavior};

use tokio_util::sync::CancellationToken;

fn debug_enabled() -> bool {
    std::env::var("DEBUG").ok().as_deref() == Some("1")
}

#[tokio::main]
async fn main() {
    // Global panic handler — log with backtrace then let process exit
    // so the crash-restart wrapper can bring it back up.
    std::panic::set_hook(Box::new(|info| {
        let location = info.location()
            .map(|l| format!("{}:{}", l.file(), l.line()))
            .unwrap_or_else(|| "unknown".to_string());
        let payload = info.payload()
            .downcast_ref::<&str>()
            .copied()
            .unwrap_or("non-string panic");
        eprintln!("PANIC at {}: {}", location, payload);
        eprintln!("{:?}", std::backtrace::Backtrace::capture());
    }));

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
    let mut tars_positions: Vec<crate::tars::Position> = Vec::new();

    // -------------------------
    // DB (one canonical open)
    // -------------------------
    eprintln!("DB sqlite_path = {}", cfg.sqlite_path);
    let mut db = crate::db::Db::open(&cfg.sqlite_path).expect("db open failed");

    // -------------------------
    // PumpPortal stream
    // -------------------------
    let (pump_tx, mut pump_rx) = mpsc::channel::<pumpportal::types::PumpMint>(10_000);
    let (trade_tx, mut trade_rx) = mpsc::channel::<pumpportal::types::PumpTrade>(50_000);
    let (enrich_result_tx, mut enrich_result_rx) = mpsc::channel::<enrichment::EnrichmentResult>(1_000);
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
            pumpportal::client::run(cfg_pp, pump_tx, trade_tx, gov_pp).await;
        }
    });

    // Watch channel: main loop pushes active list every tick; background tasks read it
    let (active_tx, active_rx_per_coin) =
        tokio::sync::watch::channel(Vec::<String>::new());

    // Watch channel: pair addresses for onchain event ingestion
    let (tracked_tx, tracked_rx_onchain) =
        tokio::sync::watch::channel(Vec::<String>::new());

    // Background task 1: wallet learning (every 3 minutes)
    tokio::spawn({
        let cfg = cfg.clone();
        let gov = gov.clone();
        let shutdown = shutdown.clone();
        let db_path = cfg.sqlite_path.clone();
        async move {
            let mut iv = tokio::time::interval(Duration::from_secs(180));
            iv.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = iv.tick() => {
                        if let Ok(mut bg_db) = crate::db::Db::open(&db_path) {
                            let mut bg_coins: HashMap<String, CoinState> = HashMap::new();
                            let wallets = build_wallet_learning_list(&cfg, &mut bg_db, 12);
                            if !wallets.is_empty() {
                                let _ = crate::helius::ingest::ingest_wallet_activity(
                                    &cfg, &mut bg_db, &mut bg_coins,
                                    &wallets, gov.clone(), &shutdown,
                                ).await;
                            }
                        }
                    }
                }
            }
        }
    });

    // Background task 2: per-coin ingest (every 45 seconds)
    tokio::spawn({
        let cfg = cfg.clone();
        let gov = gov.clone();
        let shutdown = shutdown.clone();
        let db_path = cfg.sqlite_path.clone();
        let mut active_rx = active_rx_per_coin;
        async move {
            let mut iv = tokio::time::interval(Duration::from_secs(45));
            iv.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = iv.tick() => {
                        let top10: Vec<String> = active_rx.borrow().clone()
                            .into_iter().take(10).collect();
                        if top10.is_empty() { continue; }
                        if let Ok(mut bg_db) = crate::db::Db::open(&db_path) {
                            let mut bg_coins: HashMap<String, CoinState> = HashMap::new();
                            let _ = crate::helius::per_coin::ingest_pairs_for_active(
                                &cfg, &mut bg_db, &mut bg_coins, &top10, gov.clone(),
                            ).await;
                        }
                    }
                }
            }
        }
    });

    // Background task 3: onchain events ingest (every 45 seconds)
    tokio::spawn({
        let cfg = cfg.clone();
        let gov = gov.clone();
        let shutdown = shutdown.clone();
        let db_path = cfg.sqlite_path.clone();
        let mut tracked_rx = tracked_rx_onchain;
        async move {
            let mut iv = tokio::time::interval(Duration::from_secs(45));
            iv.set_missed_tick_behavior(MissedTickBehavior::Skip);
            loop {
                tokio::select! {
                    _ = shutdown.cancelled() => break,
                    _ = iv.tick() => {
                        let tracked: Vec<String> = tracked_rx.borrow().clone();
                        if tracked.is_empty() { continue; }
                        if let Ok(mut bg_db) = crate::db::Db::open(&db_path) {
                            let mut bg_coins: HashMap<String, CoinState> = HashMap::new();
                            let _ = crate::scoring::onchain::fetch_onchain_events(
                                &cfg, &mut bg_db, &mut bg_coins, &tracked, gov.clone(), &shutdown,
                            ).await;
                        }
                    }
                }
            }
        }
    });

    println!(
        "{}🚀 Solana Meme Sniper started{} (Ctrl+C to stop)",
        fmt::NEON_GREEN,
        fmt::RESET
    );
    println!("\x1b[35m● INTERFECTOR ONLINE — watching for signals...\x1b[0m");

    // Animator task — shows live status on the terminal status line
    let last_tick_ts: Arc<AtomicI64> = Arc::new(AtomicI64::new(0));
    {
        let shutdown = shutdown.clone();
        let last_tick = last_tick_ts.clone();
        tokio::spawn(async move {
            let hunting_frames = [
                "🩷 INTERFECTOR HUNTING",
                "🩷🩷 INTERFECTOR HUNTING",
                "🩷🩷🩷 INTERFECTOR HUNTING",
                "💗 INTERFECTOR HUNTING",
                "💓 INTERFECTOR HUNTING",
                "💕 INTERFECTOR HUNTING",
            ];
            let mut i = 0usize;
            loop {
                if shutdown.is_cancelled() {
                    print!("\r\x1b[K");
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                    break;
                }

                let last = last_tick.load(Ordering::Relaxed);
                let now = chrono::Utc::now().timestamp();
                let secs = now - last;

                if last == 0 {
                    print!("\r\x1b[35m🌊 INTERFECTOR INITIALIZING...\x1b[0m  ");
                } else if secs > 200 {
                    // Dead — 3 full minutes with no tick
                    print!("\r\x1b[31m💀 INTERFECTOR DEAD — {}s — run ./slime\x1b[0m  ", secs);
                } else if secs > 15 {
                    // Helius is slow but alive — show seconds so you know progress
                    print!("\r\x1b[35m🩷 INTERFECTOR HUNTING — helius {}s\x1b[0m  ", secs);
                } else {
                    // Healthy
                    let frame = hunting_frames[i % hunting_frames.len()];
                    print!("\r\x1b[35m{}\x1b[0m  ", frame);
                }

                let _ = std::io::Write::flush(&mut std::io::stdout());
                i += 1;
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            }
        });
    }

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
                last_tick_ts.store(now_ts, Ordering::Relaxed);

                // ------------------------------------------------------------
                // 1) Drain PumpPortal mints (fast path)
                // ------------------------------------------------------------
                drain_pump_mints(&mut pump_rx, &mut coins, &mut discovered, &mut market, &shutdown, &mut db);

                // ------------------------------------------------------------
                // 1b) Drain PumpPortal trades → real-time wallet events
                //     This replaces Helius for pre-graduation coins
                // ------------------------------------------------------------
                drain_pump_trades(&mut trade_rx, &mut coins, now, &shutdown);

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
                        &calls,
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

                    // Drain enrichment results back into CoinState
                    while let Ok(result) = enrich_result_rx.try_recv() {
                        if let Some(st) = coins.get_mut(&result.mint) {
                            st.holder_count = result.holder_count;
                            st.top_holder_pct = result.top_holder_pct;
                            st.is_graduated = result.is_graduated;
                            st.dex_has_socials = result.dex_has_socials;
                            st.dex_boost_active = result.dex_boost_active;
                            st.enrichment_done = true;

                            // If top holder > 40% that's a rug red flag — kill the score
                            if result.top_holder_pct.unwrap_or(0.0) > 0.40 {
                                eprintln!(
                                    "🚩 TOP_HOLDER_CONC mint={} top1={:.0}%",
                                    &result.mint[..8.min(result.mint.len())],
                                    result.top_holder_pct.unwrap_or(0.0) * 100.0
                                );
                                st.score = -999;
                            }
                        }
                    }

                    // Trigger enrichment for active coins that haven't been enriched yet
                    let unenriched: Vec<String> = active.iter()
                        .filter(|m| coins.get(*m).map(|s| !s.enrichment_done).unwrap_or(false))
                        .cloned()
                        .take(5) // max 5 per tick to avoid API spam
                        .collect();

                    if !unenriched.is_empty() {
                        // Mark as in-flight so we don't re-dispatch
                        for m in &unenriched {
                            if let Some(st) = coins.get_mut(m) {
                                st.enrichment_done = true; // will be overwritten when result arrives
                            }
                        }
                        let rpc = cfg.helius_rpc_url.clone();
                        let tx = enrich_result_tx.clone();
                        tokio::spawn(async move {
                            enrichment::run_enrichment_batch(unenriched, rpc, tx).await;
                        });
                    }

                    // Share active list and pair addresses with background ingest tasks
                    active_tx.send(active.clone()).ok();
                    let tracked = build_tracked_pair_addresses(&coins, &active, &queue, 50);
                    tracked_tx.send(tracked).ok();

                    while let Ok(msg) = tg_rx.try_recv() {
                        let token = cfg.telegram_bot_token.clone();
                        let chat_id = cfg.telegram_chat_id.clone();
                        tokio::spawn(async move {
                            telegram::send_alert(&token, &chat_id, &msg).await;
                        });
                    }
                }

                // TARS auto-buy on new calls
                if cfg.tars_enabled {
                    eprintln!("DBG TARS: enabled=true calls={} positions={}", calls.len(), tars_positions.len());
                    // open position for each new call
                    for call in calls.iter().rev().take(1) {
                        eprintln!("DBG TARS: checking call mint={} market_has={}", &call.mint[..8], market.map.contains_key(&call.mint));
                        if let Some(ms) = market.map.get(&call.mint) {
                            if let Some(fdv) = ms.fdv {
                                let already_open = tars_positions.iter()
                                    .any(|p| p.mint == call.mint && !p.closed);
                                eprintln!("DBG TARS: fdv={:.0} already_open={}", fdv, already_open);

                                if !already_open {
                                    let pub_key = cfg.pumpportal_public_key.clone();
                                    let priv_key = cfg.pumpportal_private_key.clone();
                                    let mint = call.mint.clone();
                                    let sol = cfg.tars_buy_sol;
                                    let rpc = cfg.helius_rpc_url.clone();

                                    tokio::spawn(async move {
                                        match crate::tars::buy(&pub_key, &priv_key, &mint, sol, &rpc).await {
                                            Ok(sig) => println!("🤖 TARS BUY {} {:.2} SOL sig={}",
                                                &mint[..8], sol, &sig[..8]),
                                            Err(e) => eprintln!("🤖 TARS BUY ERR={:?}", e),
                                        }
                                    });

                                    tars_positions.push(crate::tars::Position::new(
                                        &call.mint, fdv, sol, now_ts
                                    ));

                                    // Write to DB for dashboard
                                    let _ = db.upsert_tars_position(&call.mint, fdv, fdv, sol, now_ts);

                                    println!("🤖 TARS opened position: {} entry=${:.0}",
                                        &call.mint[..8], fdv);
                                }
                            }
                        }
                    }
                }

                // TARS position monitor
                if cfg.tars_enabled && !tars_positions.is_empty() {
                    let mut i = 0;
                    while i < tars_positions.len() {
                        let pos = &mut tars_positions[i];
                        if pos.closed {
                            i += 1;
                            continue;
                        }

                        // get current FDV from market cache
                        if let Some(ms) = market.map.get(&pos.mint) {
                            if let Some(fdv) = ms.fdv {
                                // keep dashboard in sync
                                let _ = db.update_tars_position_fdv(&pos.mint, fdv);

                                let signal = pos.check_exits(
                                    fdv,
                                    cfg.tars_tp1_mult,
                                    cfg.tars_tp2_mult,
                                    cfg.tars_sl_pct,
                                    now_ts,
                                );

                                match signal {
                                    Some(crate::tars::ExitSignal::TakeProfit1) => {
                                        let mult = fdv / pos.entry_fdv;
                                        println!("🤖 TARS TP1 {:.2}x — selling 50%", mult);
                                        let _ = db.update_tars_position_fdv(&pos.mint, fdv);
                                        let pub_key = cfg.pumpportal_public_key.clone();
                                        let priv_key = cfg.pumpportal_private_key.clone();
                                        let mint = pos.mint.clone();
                                        let rpc = cfg.helius_rpc_url.clone();
                                        tokio::spawn(async move {
                                            match crate::tars::sell_percent(&pub_key, &priv_key, &mint, 50.0, &rpc).await {
                                                Ok(sig) => println!("🤖 TARS TP1 sold 50% sig={}", &sig[..8]),
                                                Err(e) => eprintln!("🤖 TARS TP1 ERR={:?}", e),
                                            }
                                        });
                                    }
                                    Some(crate::tars::ExitSignal::TakeProfit2) => {
                                        println!("🤖 TARS TP2 {:.2}x — selling 30%", fdv / pos.entry_fdv);
                                        let pub_key = cfg.pumpportal_public_key.clone();
                                        let priv_key = cfg.pumpportal_private_key.clone();
                                        let mint = pos.mint.clone();
                                        let rpc = cfg.helius_rpc_url.clone();
                                        tokio::spawn(async move {
                                            match crate::tars::sell_percent(&pub_key, &priv_key, &mint, 30.0, &rpc).await {
                                                Ok(sig) => println!("🤖 TARS TP2 sold 30% sig={}", &sig[..8]),
                                                Err(e) => eprintln!("🤖 TARS TP2 ERR={:?}", e),
                                            }
                                        });
                                    }
                                    Some(crate::tars::ExitSignal::TakeProfit3) => {
                                        println!("🤖 TARS TP3 — selling 15%, keeping 5% moon bag 🌙");
                                        let pub_key = cfg.pumpportal_public_key.clone();
                                        let priv_key = cfg.pumpportal_private_key.clone();
                                        let mint = pos.mint.clone();
                                        let rpc = cfg.helius_rpc_url.clone();
                                        tokio::spawn(async move {
                                            match crate::tars::sell_percent(&pub_key, &priv_key, &mint, 15.0, &rpc).await {
                                                Ok(sig) => println!("🤖 TARS TP3 sold 15% sig={}", &sig[..8]),
                                                Err(e) => eprintln!("🤖 TARS TP3 ERR={:?}", e),
                                            }
                                        });
                                    }
                                    Some(crate::tars::ExitSignal::StopLoss) => {
                                        let mult = fdv / pos.entry_fdv;
                                        println!("🛑 TARS SL {:.2}x — selling 95%, keeping 5% moon bag 🌙", mult);
                                        let _ = db.close_tars_position(&pos.mint, "StopLoss", now_ts);
                                        let pub_key = cfg.pumpportal_public_key.clone();
                                        let priv_key = cfg.pumpportal_private_key.clone();
                                        let mint = pos.mint.clone();
                                        let rpc = cfg.helius_rpc_url.clone();
                                        tokio::spawn(async move {
                                            match crate::tars::sell_percent(&pub_key, &priv_key, &mint, 95.0, &rpc).await {
                                                Ok(sig) => println!("🛑 TARS SL sold 95% sig={}", &sig[..8]),
                                                Err(e) => eprintln!("🛑 TARS SL ERR={:?}", e),
                                            }
                                        });
                                    }
                                    None => {}
                                }
                            }
                        }
                        i += 1;
                    }
                    // clean up closed positions
                    tars_positions.retain(|p| !p.closed);
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

                        // Pink pulsing dots — cycles through 1-4 dots
                        let dot_count = ((now_ts % 4) + 1) as usize;
                        let dots: String = "●".repeat(dot_count);
                        let spaces: String = "○".repeat(4 - dot_count);

                        println!(
                            "\x1b[35m{}{}\x1b[0m {}🫀 hb{} local={} coins={} active={} queue={} calls={} discovered={}",
                            dots,
                            spaces,
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
    pump_rx: &mut mpsc::Receiver<pumpportal::types::PumpMint>,
    coins: &mut HashMap<String, CoinState>,
    discovered: &mut VecDeque<String>,
    market: &mut MarketCache,
    shutdown: &CancellationToken,
    db: &mut crate::db::Db,
) {
    if shutdown.is_cancelled() {
        return;
    }

    let mut pump_added = 0usize;

    while let Ok(pm) = pump_rx.try_recv() {
        if shutdown.is_cancelled() {
            return;
        }

        let mint = pm.mint.clone();

        // Inject initial FDV from pump data before first DexScreener poll
        // marketCapSol * SOL_PRICE_USD ≈ FDV. Use 130.0 as a rough SOL price.
        if let Some(mc_sol) = pm.market_cap_sol {
            let fdv_usd = mc_sol * 89.0;
            market.inject_pump_fdv(&mint, fdv_usd);
        }

        if !coins.contains_key(&mint) {
            let mut st = CoinState::new();

            // Store launch SOL + bonding curve % (pump.fun graduates at 85 SOL)
            st.launch_sol = pm.v_sol_in_bonding_curve;
            if let Some(v_sol) = pm.v_sol_in_bonding_curve {
                st.bonding_curve_pct = Some((v_sol / 85.0 * 100.0).min(100.0));
            }

            // Store creator wallet + flag if it's a known rugger
            if let Some(ref creator) = pm.creator {
                let is_rug = db.wallet_score(creator).unwrap_or(0) <= -50;
                st.creator_wallet = Some(creator.clone());
                st.creator_is_rug = is_rug;
                if is_rug {
                    eprintln!(
                        "🚩 RUG CREATOR detected at launch: mint={} creator={}",
                        &mint[..8.min(mint.len())],
                        &creator[..8.min(creator.len())]
                    );
                }
            }

            // Social quality signal — prepped projects run harder
            st.social_score = pm.meta.social_score();
            st.has_socials = pm.meta.has_socials();

            coins.insert(mint.clone(), st);
            discovered.push_back(mint);
            pump_added += 1;

            while discovered.len() > 20_000 {
                discovered.pop_front();
            }
        } else {
            // Coin already known — update bonding curve data if available
            if let Some(st) = coins.get_mut(&mint) {
                if let Some(v_sol) = pm.v_sol_in_bonding_curve {
                    st.launch_sol = Some(v_sol);
                    st.bonding_curve_pct = Some((v_sol / 85.0 * 100.0).min(100.0));
                }
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
    calls: &Vec<CallRecord>,
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
    for call in calls.iter() {
        let call_age = now_ts.saturating_sub(call.ts as i64);
        if call_age <= 3600 {
            if seen.insert(call.mint.clone()) {
                mints.push(call.mint.clone());
            }
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
        let mem_signers = coins.get(&mint).map(|s| s.unique_signers_5m as u64).unwrap_or(0);
        let db_signers = db.signers_5m(now_ts, mint.as_str()).unwrap_or(0);
        let signers = mem_signers.max(db_signers);
        let first_seen = coins.get(&mint).map(|s| s.first_seen).unwrap_or(0);

        if fdv.unwrap_or(0.0) <= 0.0 && tx5.unwrap_or(0) == 0 && signers == 0 {
            continue;
        }

        let ev = db.events_5m(now_ts, mint.as_str()).unwrap_or(0) as usize;
        let is_active = active_set.contains(mint.as_str());

        let _ = db.insert_snapshot(
            now_ts, mint.as_str(), fdv, tx5, score, signers, ev, first_seen, is_active, false,
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

fn drain_pump_trades(
    trade_rx: &mut mpsc::Receiver<pumpportal::types::PumpTrade>,
    coins: &mut HashMap<String, CoinState>,
    now: u64,
    shutdown: &CancellationToken,
) {
    if shutdown.is_cancelled() { return; }

    let mut ingested = 0usize;

    while let Ok(trade) = trade_rx.try_recv() {
        if shutdown.is_cancelled() { return; }

        // Only track coins we're already watching
        let Some(st) = coins.get_mut(&trade.mint) else { continue };

        // Build real-time Event from trade
        let tier = crate::types::WhaleTier::None; // tier enrichment happens in helius, not here

        let event = crate::types::Event {
            wallet: trade.trader.clone(),
            ts: trade.ts,
            sol: if trade.is_buy { trade.sol_amount } else { -trade.sol_amount },
            tier,
        };

        st.events.push(event);
        st.last_activity_ts = now;

        // Update market cap if available
        // (gives us a free FDV estimate between DexScreener polls)
        if let Some(mc_sol) = trade.market_cap_sol {
            // Will be picked up by scoring on next tick
            let _ = mc_sol; // market cache updated separately
        }

        ingested += 1;
    }

    if ingested > 0 {
        if std::env::var("DEBUG").ok().as_deref() == Some("1") {
            eprintln!("DBG pump_trades ingested={}", ingested);
        }
    }
}

fn build_wallet_learning_list(
    cfg: &crate::config::Config,
    db: &mut crate::db::Db,
    limit: usize,
) -> Vec<String> {
    // Prefer .env wallets first — these are your elite hand-picked wallets
    let env_wallets: Vec<String> = cfg
        .helius_wallets
        .split(',')
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .collect();

    if !env_wallets.is_empty() {
        return env_wallets;
    }

    // Fall back to DB watchlist if no .env wallets
    db.get_watchlist_wallets(limit)
}
