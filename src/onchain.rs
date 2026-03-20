use crate::config::Config;
use crate::helius::client::fetch_address_txs;
use crate::helius::parse::{classify_tier, collect_mints, estimate_sol_outflow};
use crate::types::{CoinState, Event};
use std::collections::HashMap;

pub async fn fetch_onchain_events(cfg: &Config, coins: &mut HashMap<String, CoinState>) {
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
