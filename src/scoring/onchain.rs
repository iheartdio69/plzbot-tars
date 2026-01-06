// onchain.rs (new)
use crate::config::Config;
use crate::types::{CoinState, Event, WhaleTier};
use helius::Helius;
use helius::types::Cluster;
use solana_sdk::signature::Signature;
use std::collections::HashMap;

pub async fn fetch_onchain_events(cfg: &Config, coins: &mut HashMap<String, CoinState>) {
    let helius = Helius::new(&cfg.helius_api_key, Cluster::MainnetBeta).unwrap();

    for (mint, state) in coins.iter_mut() {
        let signatures = helius.connection().get_signatures_for_address(mint, None).unwrap_or_default();
        for sig_info in signatures {
            let sig = Signature::from_str(&sig_info.signature).unwrap();
            let tx = helius.connection().get_transaction(&sig.to_string(), None).unwrap_or_default();

            if tx.transaction.message.instructions.iter().any(|i| i.program_id == cfg.pump_fun_program) {
                // Simplistic parse
                let wallet = tx.transaction.message.account_keys[0].to_string(); // First signer
                let sol_delta = tx.meta.pre_balances[0] - tx.meta.post_balances[0]; // Rough SOL proxy (lamports)
                let sol = sol_delta as f64 / 1_000_000_000.0;
                let tier = if sol > cfg.blue_sol_tx { WhaleTier::Blue } else if sol > cfg.beluga_sol_tx { WhaleTier::Beluga } else { WhaleTier::None };
                state.events.push(Event {
                    wallet,
                    ts: tx.block_time.unwrap_or(0) as u64,
                    sol,
                    tier,
                });
            }
        }
    }
}
