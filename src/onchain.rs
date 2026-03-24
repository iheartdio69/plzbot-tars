use crate::config::Config;
use crate::helius::client::fetch_address_txs;
use crate::market::cache::MarketCache;
use crate::helius::parse::{classify_tier, collect_mints, estimate_sol_outflow};
use crate::types::{CoinState, Event};
use std::collections::HashMap;
use std::time::{Duration, Instant};

/// How often we hit Helius at all — saves credits vs. every-tick polling
const ONCHAIN_INTERVAL_SECS: u64 = 60;

/// Minimum buys in 5m for a coin to be worth enriching with on-chain data
const MIN_BUYS_FOR_ONCHAIN: u64 = 5;

static LAST_ONCHAIN_RUN: std::sync::OnceLock<std::sync::Mutex<Option<Instant>>> =
    std::sync::OnceLock::new();

pub async fn fetch_onchain_events(
    cfg: &Config,
    coins: &mut HashMap<String, CoinState>,
    market: &MarketCache,
) {
    // Rate-limit: only run every ONCHAIN_INTERVAL_SECS
    let lock = LAST_ONCHAIN_RUN.get_or_init(|| std::sync::Mutex::new(None));
    {
        let mut last = lock.lock().unwrap();
        if let Some(t) = *last {
            if t.elapsed() < Duration::from_secs(ONCHAIN_INTERVAL_SECS) {
                return;
            }
        }
        *last = Some(Instant::now());
    }

    // Build set of mints that have DexScreener momentum — only enrich these
    // Saves credits by ignoring dead/stale coins
    let active_mints: std::collections::HashSet<String> = market.map
        .iter()
        .filter(|(_, samples)| {
            // Check latest snapshot for activity
            if let Some(latest) = samples.last() {
                let buys = latest.buys_5m.unwrap_or(0);
                let sells = latest.sells_5m.unwrap_or(0);
                let total = buys + sells;
                let price_change = latest.price_change_5m.unwrap_or(0.0).abs();
                buys >= MIN_BUYS_FOR_ONCHAIN || (total >= 3 && price_change > 2.0)
            } else {
                false
            }
        })
        .map(|(mint, _)| mint.clone())
        .collect();

    if active_mints.is_empty() {
        return;
    }

    let wallets: Vec<String> = std::env::var("HELIUS_WALLETS")
        .unwrap_or_default()
        .split(',')
        .map(|s| s.trim().to_string())
        .filter(|s| !s.is_empty())
        .collect();

    for wallet in &wallets {
        let txs = match fetch_address_txs(
            &cfg.helius_addr_url,
            &cfg.helius_api_key,
            wallet,
            cfg.fetch_limit,
        )
        .await
        {
            Ok(t) => t,
            Err(_) => continue,
        };

        for tx in txs {
            let ts = tx.timestamp.unwrap_or(0);
            let actor = tx
                .token_transfers
                .first()
                .and_then(|t| t.from_user_account.clone())
                .unwrap_or_else(|| "UNKNOWN".to_string());
            let sol = estimate_sol_outflow(&tx.native_transfers, &actor);
            let tier = classify_tier(sol, cfg);
            let mints = collect_mints(&tx.token_transfers, cfg);

            for mint in mints {
                // Only enrich coins that are showing DexScreener momentum
                if !active_mints.contains(&mint) {
                    continue;
                }
                if let Some(state) = coins.get_mut(&mint) {
                    state.events.push(Event {
                        wallet: actor.clone(),
                        ts,
                        sol,
                        tier,
                    });
                }
            }
        }
    }
}
