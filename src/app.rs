use crate::config::Config;
use std::time::Instant;
use crate::helius::websocket::{subscribe_pump_fun, NewMintsSink};
use crate::market::cache::MarketCache;
use crate::market::discovery::{merge_discovered, MarketDiscovery};
use crate::missed_calls::MissedCallTracker;
use crate::onchain::fetch_onchain_events;
use crate::resolver::resolve_calls;
use crate::rug_tracker::{apply_to_reputation, load_rug_tracker, WalletStrike};
use crate::scoring::engine::score_and_manage;
use crate::scoring::shadow::ShadowMap;
use crate::types::{CallRecord, CoinState};
use std::collections::{HashMap, VecDeque};
use std::fs;
use std::sync::{Arc, Mutex};
use std::time::Duration;

pub async fn run(cfg: Config) {
    let mut coins: HashMap<String, CoinState> = HashMap::new();
    let mut active: Vec<String> = Vec::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    let mut calls: Vec<CallRecord> = Vec::new();
    let mut market = MarketCache::default();
    let mut discovered: VecDeque<String> = VecDeque::new();
    let mut discovery = MarketDiscovery::default();
    let mut shadow: ShadowMap = HashMap::new();
    let mut rug_tracker: HashMap<String, WalletStrike> = load_rug_tracker();
    let mut missed_tracker = MissedCallTracker::load();
    let mut live_positions: Vec<crate::trading::position::Position> =
        crate::trading::position::load_positions();
    apply_to_reputation(&rug_tracker);
    println!("🗂️  Rug tracker loaded: {} wallets", rug_tracker.len());

    // Helius WebSocket — catches new pump.fun mints at birth
    let ws_sink: NewMintsSink = Arc::new(Mutex::new(Vec::new()));
    let ws_sink_bg = ws_sink.clone();
    let cfg_ws = cfg.clone();
    tokio::spawn(async move {
        subscribe_pump_fun(&cfg_ws, ws_sink_bg).await;
    });
    println!("🔌 Helius WebSocket spawned — catching mints at birth");

    loop {
        println!(
            "🫀 tick coins={} active={} calls={} market_cache={} discovered={}",
            coins.len(),
            active.len(),
            calls.len(),
            market.map.len(),
            discovered.len()
        );

        // ── INSTANT SEED DRAIN — every tick ───────────────────────────
        // Reads seed_mints.json on every loop so Lab/WATCHER mints are processed
        // within 1 second instead of waiting up to 60s for discovery cycle
        {
            let seed_path = "data/seed_mints.json";
            if let Ok(s) = std::fs::read_to_string(seed_path) {
                if let Ok(seeds) = serde_json::from_str::<Vec<String>>(&s) {
                    let mut added = 0usize;
                    for mint in &seeds {
                        if !coins.contains_key(mint) {
                            coins.insert(mint.clone(), crate::types::CoinState::new_with_mint(mint.clone()));
                            added += 1;
                        }
                    }
                    if added > 0 {
                        println!("🌱 Seed drain: {} new mints", added);
                        // Clear after loading
                        let _ = std::fs::write(seed_path, "[]");
                    }
                }
            }
        }

        // Drain WebSocket new mints into coins
        {
            let mut new_mints = ws_sink.lock().unwrap();
            let mut ws_added = 0usize;
            for mint in new_mints.drain(..) {
                coins.entry(mint.clone()).or_insert_with(|| {
                    ws_added += 1;
                    CoinState::new_with_mint(mint)
                });
            }
            if ws_added > 0 {
                println!("⚡ WebSocket: {} new mints added", ws_added);
                // Forward latest WS mint to POOR TODAY via TARSFEEDBOT
                if let Some(mint) = coins.keys().next() {
                    let mint_str = mint.clone();
                    let _ = crate::telegram::send_message(
                        "8664564592:AAHtxurnWgJ6EqizNq0_zxubJTu6HsHsyhI",
                        "-1003740417472",
                        &mint_str,
                    ).await;
                }
            }
        }

        if discovery.should_run(&cfg) {
            let new_mints: Vec<String> = discovery.run(&cfg).await;
            let added = merge_discovered(&mut discovered, new_mints.clone(), 200);
            println!(
                "Discovered {} new mints (added {})",
                new_mints.len(),
                added
            );
            for mint in new_mints {
                coins
                    .entry(mint.clone())
                    .or_insert_with(|| CoinState::new_with_mint(mint));
            }
        }

        // Poll active coins every single tick — as fast as possible
        if !active.is_empty() {
            market.poll_active(&active).await;
        }

        // Poll called coins every second — hawk mode, money on the line
        if !calls.is_empty() {
            let called_mints: Vec<String> = calls.iter().map(|c| c.mint.clone()).collect();
            market.poll_called(&called_mints).await;

            // Update paper wallet trades with latest prices
            for call in &calls {
                let trend = crate::market::cache::market_trend(&market, &call.mint, &cfg);
                if let Some(fdv) = trend.last_fdv {
                    if fdv > 0.0 {
                        crate::trading::paper_wallets::update_paper_trades(&call.mint, fdv);
                    }
                }
            }
        }

        // ── LIVE POSITION EXIT MONITOR ────────────────────────────────
        if cfg.tars_enabled && !live_positions.is_empty() {
            let mut positions_changed = false;

            for pos in live_positions.iter_mut() {
                if pos.status == crate::trading::position::PositionStatus::Closed {
                    continue;
                }
                let trend = crate::market::cache::market_trend(&market, &pos.mint, &cfg);
                let current_fdv = match trend.last_fdv {
                    Some(f) if f > 0.0 => f,
                    _ => continue,
                };

                // Update peak
                if current_fdv > pos.peak_fdv {
                    pos.peak_fdv = current_fdv;
                    pos.peak_mult = current_fdv / pos.entry_fdv;
                }

                match pos.check_thresholds(current_fdv) {
                    crate::trading::position::PositionAction::ExitFull(reason) => {
                        println!("  🔴 EXIT {} | {} | mult {:.2}x",
                            &pos.mint[..8], reason, pos.peak_mult);
                        // Sell 95%, keep 5% moon bag forever
                        match crate::trading::pumpportal::sell(
                            &cfg.tars_wallet_pubkey, &cfg.tars_private_key,
                            &cfg.pumpportal_api_key, &pos.mint, 95.0, &cfg.helius_rpc_url,
                        ).await {
                            Ok(sig) => {
                                pos.outcome = Some(if pos.peak_mult >= 1.5 { "WIN".into() } else { "LOSS".into() });
                                pos.status = crate::trading::position::PositionStatus::Closed;
                                positions_changed = true;
                                if !cfg.telegram_bot_token.is_empty() {
                                    let icon = if pos.peak_mult >= 1.5 { "✅" } else { "❌" };
                                    crate::telegram::send_message(
                                        &cfg.telegram_bot_token,
                                        &cfg.telegram_chat_id,
                                        &format!("{} <b>SOLD</b> {:.2}x\n{}\n<code>{}</code>",
                                            icon, pos.peak_mult, reason, &sig[..16]),
                                    ).await;
                                }
                            }
                            Err(e) => println!("  ❌ SELL FAILED: {}", e),
                        }
                    }
                    crate::trading::position::PositionAction::ExitPartial(pct, reason) => {
                        println!("  🟡 PARTIAL EXIT {:.0}% {} | {}", pct, &pos.mint[..8], reason);
                        match crate::trading::pumpportal::sell(
                            &cfg.tars_wallet_pubkey, &cfg.tars_private_key,
                            &cfg.pumpportal_api_key, &pos.mint, pct, &cfg.helius_rpc_url,
                        ).await {
                            Ok(sig) => {
                                pos.tp1_triggered = true;
                                positions_changed = true;
                                if !cfg.telegram_bot_token.is_empty() {
                                    crate::telegram::send_message(
                                        &cfg.telegram_bot_token,
                                        &cfg.telegram_chat_id,
                                        &format!("🟡 <b>PARTIAL SELL {:.0}%</b> {:.2}x\n{}\n<code>{}</code>",
                                            pct, pos.peak_mult, reason, &sig[..16]),
                                    ).await;
                                }
                            }
                            Err(e) => println!("  ❌ PARTIAL SELL FAILED: {}", e),
                        }
                    }
                    crate::trading::position::PositionAction::Hold => {}
                }
            }

            if positions_changed {
                crate::trading::position::save_positions(&live_positions);
            }
        }

        // Prune stale coins — active coins ride 2 hours, inactive cleared after 10 min
        coins.retain(|_, c| {
            if c.active { c.first_seen.elapsed().as_secs() < 7200 } // 2hr for active
            else { c.first_seen.elapsed().as_secs() < 600 }          // 10min for inactive
        });
        active.retain(|m| coins.contains_key(m));

        // Poll all other coins on normal cadence — newest first
        if market.last_poll.elapsed().as_secs() >= cfg.market_poll_secs {
            market.last_poll = Instant::now();
            let mut mint_list: Vec<String> = coins.keys().cloned().collect();
            // Sort newest first so fresh coins get data priority
            mint_list.sort_by(|a, b| {
                let age_a = coins.get(a).map(|c| c.first_seen.elapsed().as_secs()).unwrap_or(9999);
                let age_b = coins.get(b).map(|c| c.first_seen.elapsed().as_secs()).unwrap_or(9999);
                age_a.cmp(&age_b)
            });
            market.poll(&cfg, &mint_list).await;
        }
        fetch_onchain_events(&cfg, &mut coins, &market).await;
        let prev_call_count = calls.len();
        score_and_manage(
            &cfg,
            &mut coins,
            &mut active,
            &mut queue,
            &mut calls,
            &market,
            &mut shadow,
            &mut missed_tracker,
        ).await;

        // When a new call is made — remove from active (graduated) + open slot + alert
        if calls.len() > prev_call_count {
            for call in &calls[prev_call_count..] {
                // Graduate: remove from active list so the slot opens up
                active.retain(|m| m != &call.mint);
                if let Some(c) = coins.get_mut(&call.mint) {
                    c.active = false; // no longer competing for active slots
                }

                // Promote next from queue into active
                while active.len() < cfg.max_active_coins {
                    if let Some(next) = queue.pop_front() {
                        if !active.contains(&next) {
                            if let Some(nc) = coins.get_mut(&next) {
                                nc.active = true;
                                active.push(next.clone());
                            }
                        }
                    } else {
                        break;
                    }
                }

                // Open paper trades for all 5 strategy wallets
                crate::trading::paper_wallets::open_paper_trades(
                    &call.mint,
                    call.fdv_at_call,
                );

                // Telegram alert — details first, then raw CA for easy copy
                if !cfg.telegram_bot_token.is_empty() {
                    let trend = crate::market::cache::market_trend(&market, &call.mint, &cfg);
                    crate::telegram::send_call_alert(
                        &cfg.telegram_bot_token,
                        &cfg.telegram_chat_id,
                        call,
                        "CALL",
                        trend.last_fdv.unwrap_or(0.0),
                        trend.last_liq.unwrap_or(0.0),
                        trend.fdv_velocity_pct,
                        trend.buy_sell_ratio,
                        trend.price_change_1h,
                        0,
                        call.score,
                    ).await;
                    // Send raw CA as plain text so humans can copy it easily
                    crate::telegram::send_message(
                        &cfg.telegram_bot_token,
                        &cfg.telegram_chat_id,
                        &call.mint,
                    ).await;
                }

                // ── DIP SNIPER ENTRY ──────────────────────────────────
                if cfg.tars_enabled {
                    let trend = crate::market::cache::market_trend(&market, &call.mint, &cfg);
                    let call_fdv = trend.last_fdv.unwrap_or(call.fdv_at_call);
                    let call_velocity = trend.fdv_velocity_pct;

                    let entry = crate::trading::entry::wait_for_entry(
                        &call.mint,
                        call_fdv,
                        call_velocity,
                        &market,
                        &cfg,
                    ).await;

                    match entry {
                        crate::trading::entry::EntryDecision::Enter { fdv, reason } => {
                            println!("  ✅ ENTRY: {} | FDV ${:.0} | {}", &call.mint[..8], fdv, reason);

                            // ── WALLET 2: GUT_LOCK — buy simultaneously ──
                            if !cfg.tars_wallet2_key.is_empty() {
                                match crate::trading::pumpportal::buy(
                                    &cfg.tars_wallet2_pubkey,
                                    &cfg.tars_wallet2_key,
                                    &cfg.tars_wallet2_api_key,
                                    &call.mint,
                                    cfg.tars_sol_tx,
                                    &cfg.helius_rpc_url,
                                ).await {
                                    Ok(sig) => println!("  🔒 W2 GUT_LOCK BUY {} sig:{}", &call.mint[..8], &sig[..12]),
                                    Err(e) => println!("  ❌ W2 BUY FAILED: {}", e),
                                }
                            }

                            // ── WALLET 1: GUT_MOON — main buy ────────────
                            match crate::trading::pumpportal::buy(
                                &cfg.tars_wallet_pubkey,
                                &cfg.tars_private_key,
                                &cfg.pumpportal_api_key,
                                &call.mint,
                                cfg.tars_sol_tx,
                                &cfg.helius_rpc_url,
                            ).await {
                                Ok(sig) => {
                                    println!("  🚀 BUY executed: {} sig:{}", &call.mint[..8], &sig[..12]);

                                    // Open live position for exit monitoring
                                    let pos = crate::trading::position::Position::new(
                                        call.mint.clone(),
                                        fdv,
                                        fdv,
                                        cfg.tars_sol_tx,
                                    );
                                    live_positions.push(pos);
                                    crate::trading::position::save_positions(&live_positions);

                                    if !cfg.telegram_bot_token.is_empty() {
                                        crate::telegram::send_message(
                                            &cfg.telegram_bot_token,
                                            &cfg.telegram_chat_id,
                                            &format!("🚀 <b>BOUGHT</b>\n<code>{}</code>\nEntry FDV: ${:.0}\nReason: {}\nSig: <code>{}</code>",
                                                call.mint, fdv, reason, &sig[..16]),
                                        ).await;
                                    }
                                }
                                Err(e) => {
                                    println!("  ❌ BUY FAILED: {} — {}", &call.mint[..8], e);
                                    if !cfg.telegram_bot_token.is_empty() {
                                        crate::telegram::send_message(
                                            &cfg.telegram_bot_token,
                                            &cfg.telegram_chat_id,
                                            &format!("❌ Buy failed: {}\n{}", &call.mint[..12], e),
                                        ).await;
                                    }
                                }
                            }
                        }
                        crate::trading::entry::EntryDecision::Skip { reason } => {
                            println!("  ❌ SKIP ENTRY: {} | {}", &call.mint[..8], reason);
                            if !cfg.telegram_bot_token.is_empty() {
                                crate::telegram::send_message(
                                    &cfg.telegram_bot_token,
                                    &cfg.telegram_chat_id,
                                    &format!("❌ Entry skipped: {}\nReason: {}", &call.mint[..12], reason),
                                ).await;
                            }
                        }
                    }
                }
            }
        }

        // Resolve outcomes on existing calls
        let resolution_alerts = resolve_calls(
            &cfg,
            &market,
            &mut calls,
            &mut rug_tracker,
            &cfg.telegram_bot_token,
            &cfg.telegram_chat_id,
        );
        for (mint, msg) in resolution_alerts {
            if !cfg.telegram_bot_token.is_empty() {
                // Parse "OUTCOME|mult|reason" format
                let parts: Vec<&str> = msg.splitn(3, '|').collect();
                if parts.len() == 3 {
                    let outcome = parts[0];
                    let mult: f64 = parts[1].parse().unwrap_or(1.0);
                    let reason = parts[2];
                    crate::telegram::send_resolution(
                        &cfg.telegram_bot_token,
                        &cfg.telegram_chat_id,
                        &mint,
                        outcome,
                        mult,
                        reason,
                    ).await;
                }
            }
        }

        // Write state for the UI dashboard
        let _ = fs::create_dir_all("data");
        let state = serde_json::json!({
            "coins": coins.len(),
            "active": active,
            "calls_total": calls.len(),
            "tars_enabled": false,
            "ts": crate::time::now_ts(),
        });
        let _ = fs::write("data/state.json", state.to_string());
        let _ = fs::write(
            "data/calls.json",
            serde_json::to_string_pretty(&calls).unwrap_or_else(|_| "[]".into()),
        );

        tokio::time::sleep(Duration::from_secs(cfg.main_loop_sleep)).await;
    }
}
