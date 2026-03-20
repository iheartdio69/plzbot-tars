use crate::config::Config;
use crate::market::cache::MarketCache;
use crate::market::discovery::{merge_discovered, MarketDiscovery};
use crate::onchain::fetch_onchain_events;
use crate::scoring::engine::score_and_manage;
use crate::scoring::shadow::ShadowMap;
use crate::types::{CallRecord, CoinState};
use std::collections::{HashMap, VecDeque};
use std::fs;
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

    loop {
        println!(
            "🫀 tick coins={} active={} calls={} market_cache={} discovered={}",
            coins.len(),
            active.len(),
            calls.len(),
            market.map.len(),
            discovered.len()
        );

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

        let mint_list: Vec<String> = coins.keys().cloned().collect();
        market.poll(&cfg, &mint_list).await;
        fetch_onchain_events(&cfg, &mut coins).await;
        score_and_manage(
            &cfg,
            &mut coins,
            &mut active,
            &mut queue,
            &mut calls,
            &market,
            &mut shadow,
        );

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
