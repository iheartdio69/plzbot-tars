use crate::market::types::DexPair;
use reqwest::Client;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
struct DexSearchResponse {
    pairs: Option<Vec<DexPair>>,
}

/// Search Dexscreener for pairs.
pub async fn search_pairs(q: &str) -> Result<Vec<DexPair>, String> {
    let q_enc = urlencoding::encode(q);
    let url = format!("https://api.dexscreener.com/latest/dex/search/?q={}", q_enc);

    let client = Client::new();
    let res = client
        .get(&url)
        .send()
        .await
        .map_err(|e| format!("dexscreener request failed: {e}"))?;

    let body = res
        .json::<DexSearchResponse>()
        .await
        .map_err(|e| format!("dexscreener json parse failed: {e}"))?;

    Ok(body.pairs.unwrap_or_default())
}

/// Get one “best” pair snapshot for a mint.
/// Priority:
/// 1) Solana + baseToken.address == mint
/// 2) First Solana pair
/// 3) First pair overall
pub async fn fetch_dexscreener_snap(mint: &str) -> Option<DexPair> {
    let pairs = match search_pairs(mint).await {
        Ok(p) => p,
        Err(_) => return None,
    };

    if pairs.is_empty() {
        return None;
    }

    for p in pairs.iter() {
        if p.chain_id == "solana" && p.base_token.address == mint {
            return Some(p.clone());
        }
    }

    for p in pairs.iter() {
        if p.chain_id == "solana" {
            return Some(p.clone());
        }
    }

    pairs.into_iter().next()
}
