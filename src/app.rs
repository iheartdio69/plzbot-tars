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

        // Poll all other coins on normal cadence
        if market.last_poll.elapsed().as_secs() >= cfg.market_poll_secs {
            market.last_poll = Instant::now();
            let mint_list: Vec<String> = coins.keys().cloned().collect();
            market.poll(&cfg, &mint_list).await;
        }
        fetch_onchain_events(&cfg, &mut coins).await;
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

                // Telegram alert
                if !cfg.telegram_bot_token.is_empty() {
                    crate::telegram::send_message(
                        &cfg.telegram_bot_token,
                        &cfg.telegram_chat_id,
                        &call.mint,
                    ).await;
                    crate::telegram::send_message(
                        &cfg.telegram_bot_token,
                        &cfg.telegram_chat_id,
                        &format!(
                            "🎯 <b>CALL</b> score={}\nhttps://dexscreener.com/solana/{}",
                            call.score, call.mint
                        ),
                    ).await;
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
        for (_mint, msg) in resolution_alerts {
            if !cfg.telegram_bot_token.is_empty() {
                crate::telegram::send_message(&cfg.telegram_bot_token, &cfg.telegram_chat_id, &msg).await;
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
