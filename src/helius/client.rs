use crate::config::Config;
use crate::db::Db;
use crate::governor::Governor;
use crate::helius::types::HeliusTx;
use crate::helius::utils::{classify_tier, collect_mints, estimate_sol_outflow};
use crate::types::{CoinState, Event};

use reqwest::Client;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::Duration;

pub async fn fetch_address_txs(
    cfg: &Config,
    db: &mut Db,
    coins: &mut HashMap<String, CoinState>,
    tracked: &[String],
    gov: Arc<Governor>,
    shutdown: &tokio_util::sync::CancellationToken,
) -> Vec<String> {
    if cfg.helius_api_key.trim().is_empty() {
        return vec![];
    }

    // v0/addresses/.../transactions uses "Enhanced Transactions" credits
    let base = "https://api.helius.xyz";
    let limit: usize = 50;

    let client = Client::new();
    let mut discovered: HashSet<String> = HashSet::new();

    for addr in tracked {
        if shutdown.is_cancelled() {
            return discovered.into_iter().collect();
        }
        let url = format!(
            "{}/v0/addresses/{}/transactions?api-key={}&limit={}",
            base, addr, cfg.helius_api_key, limit
        );

        // -----------------------------
        // Request (attempt 1)
        // -----------------------------
        let permit = gov.acquire_enhanced().await;

        let mut resp = match client.get(&url).send().await {
            Ok(r) => r,
            Err(e) => {
                eprintln!("DBG helius client: send err={:?} url={}", e, url);
                // permit drops here automatically
                continue;
            }
        };

        // 429 handling + retry once
        if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
            // consume body for logging, then teach governor + backoff
            let body = resp.text().await.unwrap_or_default();
            eprintln!(
                "DBG helius client: 429 url={} body_snip={}",
                url,
                body.chars().take(200).collect::<String>()
            );

            gov.on_429_enhanced().await;

            // IMPORTANT: drop permit before sleeping/retrying to avoid inflight deadlocks
            drop(permit);

            tokio::time::sleep(Duration::from_millis(250)).await;

            // -----------------------------
            // Request (attempt 2)
            // -----------------------------
            let permit2 = gov.acquire_enhanced().await;

            resp = match client.get(&url).send().await {
                Ok(r) => r,
                Err(e2) => {
                    eprintln!("DBG helius client: retry send err={:?} url={}", e2, url);
                    // permit2 drops here
                    continue;
                }
            };

            if resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS {
                let body = resp.text().await.unwrap_or_default();
                eprintln!(
                    "DBG helius client: retry 429 url={} body_snip={}",
                    url,
                    body.chars().take(200).collect::<String>()
                );
                gov.on_429_enhanced().await;
                // permit2 drops here
                continue;
            }

            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                eprintln!(
                    "DBG helius client: retry non-200 status={} url={} body_snip={}",
                    status,
                    url,
                    body.chars().take(200).collect::<String>()
                );
                // permit2 drops here
                continue;
            }

            // success after retry
            gov.on_success(permit2.lane()).await;
            // permit2 drops at end of scope (after JSON parse)
        } else {
            // non-429 path
            if !resp.status().is_success() {
                let status = resp.status();
                let body = resp.text().await.unwrap_or_default();
                eprintln!(
                    "DBG helius client: non-200 status={} url={} body_snip={}",
                    status,
                    url,
                    body.chars().take(200).collect::<String>()
                );
                // permit drops here
                continue;
            }

            gov.on_success(permit.lane()).await;
            // permit drops at end of scope (after JSON parse)
        }

        // ✅ Only reach here with a SUCCESS `resp` that we have NOT consumed.
        let txs: Vec<HeliusTx> = match resp.json().await {
            Ok(v) => v,
            Err(e) => {
                eprintln!("DBG helius client: json err={:?} url={}", e, url);
                continue;
            }
        };

        // -----------------------------
        // Parse transactions
        // -----------------------------
        for tx in txs.iter() {
            let ts_u64: u64 = tx.timestamp.unwrap_or(0) as u64;
            let actor: String = tx.fee_payer.clone().unwrap_or_else(|| addr.clone());
            let sig = tx.signature.as_deref();

            // Collect mints once (we use it for edges + event fanout)
            let mints_in_tx: Vec<String> = collect_mints(&tx.token_transfers);

            // Discover mints + insert a best-effort edge for "actor touched mint"
            for m in &mints_in_tx {
                discovered.insert(m.clone());

                let _ = db.insert_wallet_edge(
                    ts_u64 as i64,
                    actor.as_str(),
                    None,
                    Some(m.as_str()),
                    "token_transfer",
                    None,
                    sig,
                );
            }

            // Detailed token transfer edges (from -> to)
            for tt in &tx.token_transfers {
                let mint = tt.mint.as_deref().unwrap_or("").trim();
                if mint.is_empty() {
                    continue;
                }
                let from = tt.from_user_account.as_deref().unwrap_or("").trim();
                let to = tt.to_user_account.as_deref().unwrap_or("").trim();
                if from.is_empty() || to.is_empty() {
                    continue;
                }

                let _ = db.insert_wallet_edge(
                    ts_u64 as i64,
                    from,
                    Some(to),
                    Some(mint),
                    "token_transfer",
                    None,
                    sig,
                );
            }

            // Estimate SOL outflow (native transfers)
            let sol_out = estimate_sol_outflow(&tx.native_transfers, actor.as_str());
            if sol_out > 0.0 {
                let tier = classify_tier(sol_out, cfg);

                // IMPORTANT:
                // `coins` is keyed by mint, not by wallet address.
                // So attach the event to each mint involved in this tx.
                for m in &mints_in_tx {
                    if let Some(st) = coins.get_mut(m) {
                        st.events.push(Event {
                            ts: ts_u64,
                            wallet: actor.clone(),
                            sol: sol_out,
                            tier,
                        });
                    }
                }
            }
        }
    }

    discovered.into_iter().collect()
}
