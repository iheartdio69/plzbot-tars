// src/app.rs
use crate::config::Config;
use crate::helius::client::fetch_onchain_events;
use crate::market::cache::MarketCache;
use crate::market::discovery::{merge_discovered, MarketDiscovery};
use crate::resolver::resolve_calls;
use crate::scoring::engine::score_and_manage;
use crate::scoring::shadow::ShadowMap;
use crate::types::{CallRecord, CoinState};
use std::collections::{HashMap, VecDeque};
use std::time::Duration;

pub async fn run(cfg: Config) {
    println!("🚀 solana_meme starting");
    println!(
        "CFG snapshot={}s window={}s min_call_fdv=${}",
        cfg.snapshot_interval_secs, cfg.window_secs, cfg.min_call_fdv_usd
    );

    // Bot state
    let mut coins: HashMap<String, CoinState> = HashMap::new();
    let mut active: Vec<String> = Vec::new();
    let mut queue: VecDeque<String> = VecDeque::new();
    let mut calls: Vec<CallRecord> = Vec::new();
    let mut market = MarketCache::default();
    let mut discovered: VecDeque<String> = VecDeque::new();
    let mut discovery = MarketDiscovery::default();
    let mut shadow = ShadowMap::new();

    loop {
        println!(
            "🫀 tick | coins={} active={} calls={} discovered={}",
            coins.len(),
            active.len(),
            calls.len(),
            discovered.len()
        );

        // Discovery (Dexscreener search -> new mints)
        if discovery.should_run(&cfg) {
            let new_mints = discovery.run(&cfg).await;
            let added = merge_discovered(&mut discovered, new_mints.clone(), 200);
            if added > 0 {
                println!(
                    "🕵️ Discovered {} new mints ({} added)",
                    new_mints.len(),
                    added
                );
                for mint in new_mints {
                    coins.entry(mint).or_insert_with(CoinState::new);
                }
            }
        }

        // Market poll (Dex data)
        // Poll discovered list (cap=200), try to bias toward higher FDV first
        let mut mint_list: Vec<String> = discovered.iter().cloned().collect();
        // Sort by last known FDV (desc). Missing FDV goes to bottom.
        mint_list.sort_by(|a,b| {
            let fa = market.map.get(a).and_then(|s| s.fdv).unwrap_or(0.0);
            let fb = market.map.get(b).and_then(|s| s.fdv).unwrap_or(0.0);
            fb.partial_cmp(&fa).unwrap_or(std::cmp::Ordering::Equal)
        });
        market.poll(&cfg, &mint_list).await;

        // On-chain events (Helius)
        fetch_onchain_events(&cfg, &mut coins).await;

        // Scoring / calling
        score_and_manage(
            &cfg,
            &mut coins,
            &mut active,
            &mut queue,
            &mut calls,
            &market,
            &mut shadow,
        );

        // Resolve outcomes
        resolve_calls(&cfg, &coins, &mut calls);

        tokio::time::sleep(Duration::from_secs(cfg.main_loop_sleep)).await;
    }
}
