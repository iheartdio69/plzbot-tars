/// enrichment.rs — Background data enrichment for active/queued coins
/// Pulls holder count from Helius, graduation status from Jupiter, socials from DexScreener
/// These run async in the background and write back to CoinState via shared channels

use std::collections::HashMap;
use tokio::sync::mpsc;

#[derive(Debug, Clone)]
pub struct EnrichmentResult {
    pub mint: String,
    pub holder_count: Option<u64>,
    pub top_holder_pct: Option<f64>,   // top 1 holder % of supply (rug signal)
    pub is_graduated: bool,             // found on Jupiter = liquid = graduated
    pub dex_has_socials: bool,          // DexScreener shows twitter/website
    pub dex_boost_active: u64,          // active boosts on DexScreener
}

/// Fetch holder count for a mint using Helius getTokenAccounts
pub async fn fetch_holder_count(
    mint: &str,
    rpc_url: &str,
) -> Option<(u64, f64)> {
    let client = reqwest::Client::new();

    // We page through accounts to get total count + top holder concentration
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "getTokenAccounts",
        "params": {
            "mint": mint,
            "limit": 100,
            "showZeroBalance": false
        }
    });

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client.post(rpc_url).json(&body).send()
    ).await.ok()?.ok()?;

    let data: serde_json::Value = resp.json().await.ok()?;
    let accounts = data.get("result")?.get("token_accounts")?.as_array()?;

    if accounts.is_empty() {
        return None;
    }

    let total = accounts.len() as u64;

    // Calculate top holder concentration
    let mut amounts: Vec<u128> = accounts.iter()
        .filter_map(|a| a.get("amount").and_then(|x| x.as_str()).and_then(|s| s.parse::<u128>().ok()))
        .collect();

    amounts.sort_by(|a, b| b.cmp(a));

    let total_supply: u128 = amounts.iter().sum();
    let top1_pct = if total_supply > 0 && !amounts.is_empty() {
        amounts[0] as f64 / total_supply as f64
    } else {
        0.0
    };

    Some((total, top1_pct))
}

/// Check if a mint is tradeable on Jupiter (graduation signal)
pub async fn check_jupiter_liquid(mint: &str) -> bool {
    let client = reqwest::Client::new();

    // Try to get a quote — if it works, coin is liquid
    let url = format!(
        "https://quote-api.jup.ag/v6/quote?inputMint=So11111111111111111111111111111111111111112&outputMint={}&amount=10000000&slippageBps=1000",
        mint
    );

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(4),
        client.get(&url).send()
    ).await;

    match resp {
        Ok(Ok(r)) => {
            if let Ok(data) = r.json::<serde_json::Value>().await {
                data.get("outAmount").is_some()
            } else {
                false
            }
        }
        _ => false,
    }
}

/// Fetch DexScreener social signals + boosts for a mint
pub async fn fetch_dex_signals(mint: &str) -> (bool, u64) {
    let client = reqwest::Client::new();
    let url = format!("https://api.dexscreener.com/latest/dex/tokens/{}", mint);

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(5),
        client.get(&url).send()
    ).await;

    let data: serde_json::Value = match resp {
        Ok(Ok(r)) => r.json().await.unwrap_or_default(),
        _ => return (false, 0),
    };

    let pairs = data.get("pairs").and_then(|x| x.as_array());
    let Some(pairs) = pairs else { return (false, 0) };
    let Some(pair) = pairs.first() else { return (false, 0) };

    let socials = pair.get("info")
        .and_then(|i| i.get("socials"))
        .and_then(|s| s.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);

    let websites = pair.get("info")
        .and_then(|i| i.get("websites"))
        .and_then(|w| w.as_array())
        .map(|a| !a.is_empty())
        .unwrap_or(false);

    let boosts = pair.get("boosts")
        .and_then(|b| b.get("active"))
        .and_then(|a| a.as_u64())
        .unwrap_or(0);

    (socials || websites, boosts)
}

/// Background enrichment runner — fetches data for a batch of mints
/// Sends results back via channel
pub async fn run_enrichment_batch(
    mints: Vec<String>,
    rpc_url: String,
    result_tx: mpsc::Sender<EnrichmentResult>,
) {
    for mint in mints {
        // Don't hammer APIs — small delay between mints
        tokio::time::sleep(std::time::Duration::from_millis(200)).await;

        let holders = fetch_holder_count(&mint, &rpc_url).await;
        let graduated = check_jupiter_liquid(&mint).await;
        let (dex_socials, dex_boosts) = fetch_dex_signals(&mint).await;

        let result = EnrichmentResult {
            mint: mint.clone(),
            holder_count: holders.map(|(c, _)| c),
            top_holder_pct: holders.map(|(_, p)| p),
            is_graduated: graduated,
            dex_has_socials: dex_socials,
            dex_boost_active: dex_boosts,
        };

        if graduated {
            eprintln!("🎓 GRADUATED mint={} holders={:?} boosts={}",
                &mint[..8.min(mint.len())],
                result.holder_count,
                dex_boosts
            );
        }

        if result_tx.send(result).await.is_err() {
            break;
        }
    }
}
