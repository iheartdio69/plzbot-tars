use crate::config::Config;
use crate::market::types::{is_probably_pubkey, sanitize_mint};
use serde::Deserialize;
use std::collections::{HashSet, VecDeque};
use std::time::{Duration, Instant};

#[derive(Deserialize, Debug)]
struct DexSearchResponse {
    pairs: Vec<DexPair>,
}

#[derive(Deserialize, Debug)]
struct DexPair {
    #[serde(rename = "chainId")]
    chain_id: String,
    #[serde(rename = "baseToken")]
    base_token: TokenInfo,

    fdv: Option<f64>,
    liquidity: Option<LiquidityInfo>,
    txns: Option<Txns>,

    // Dexscreener: milliseconds since epoch
    #[serde(rename = "pairCreatedAt")]
    pair_created_at: Option<u64>,
}

#[derive(Deserialize, Debug)]
struct TokenInfo {
    address: String,
}

#[derive(Deserialize, Debug)]
struct LiquidityInfo {
    usd: Option<f64>,
}

#[derive(Deserialize, Debug)]
struct Txns {
    m5: Option<TxnPeriod>,
}

#[derive(Deserialize, Debug)]
struct TxnPeriod {
    buys: Option<u64>,
    sells: Option<u64>,
}

#[derive(Default)]
pub struct MarketDiscovery {
    pub last_run: Option<Instant>,
}

impl MarketDiscovery {
    pub fn should_run(&self, cfg: &Config) -> bool {
        if !cfg.market_discovery_enabled {
            return false;
        }
        match self.last_run {
            None => true,
            Some(last) => last.elapsed() >= Duration::from_secs(cfg.market_discovery_every_secs),
        }
    }

    pub async fn run(&mut self, cfg: &Config) -> Vec<String> {
        self.last_run = Some(Instant::now());

        let mut found_mints: HashSet<String> = HashSet::new();
        let client = reqwest::Client::new();

        // age cutoff
        let now = crate::time::now(); // seconds
        let max_age = cfg.discovery_max_age_secs;
        let min_pair_created_ms = if max_age > 0 {
            Some((now.saturating_sub(max_age) as u64) * 1000)
        } else {
            None
        };

        for query in &cfg.market_discovery_queries {
            let encoded = urlencoding::encode(query);
            let url = format!(
                "https://api.dexscreener.com/latest/dex/search/?q={}",
                encoded
            );

            let resp = match client.get(&url).send().await {
                Ok(r) => r,
                Err(_) => continue,
            };

            let search_resp: DexSearchResponse = match resp.json().await {
                Ok(j) => j,
                Err(_) => continue,
            };

            for pair in search_resp.pairs.into_iter() {
                if pair.chain_id != "solana" {
                    continue;
                }

                // 24h age filter (or whatever you set)
                if let (Some(cutoff_ms), Some(created_ms)) =
                    (min_pair_created_ms, pair.pair_created_at)
                {
                    if created_ms < cutoff_ms {
                        continue;
                    }
                }

                let fdv = match pair.fdv {
                    Some(f) => f,
                    None => continue,
                };

                let liq = match pair.liquidity.and_then(|l| l.usd) {
                    Some(l) => l,
                    None => continue,
                };

                let tx_5m = pair
                    .txns
                    .and_then(|t| t.m5)
                    .map(|m| m.buys.unwrap_or(0) + m.sells.unwrap_or(0))
                    .unwrap_or(0);

                let mint = sanitize_mint(pair.base_token.address);

                if mint.is_empty() {
                    continue;
                }
                if !is_probably_pubkey(&mint) {
                    continue;
                }

                if fdv >= cfg.discovery_min_fdv_usd
                    && liq >= cfg.discovery_min_liq_usd
                    && (tx_5m as u64) >= cfg.discovery_min_tx_5m
                {
                    found_mints.insert(mint);
                }
            }
        }

        let mut mints_vec: Vec<String> = found_mints.into_iter().collect();
        mints_vec.sort();
        mints_vec
    }
}

pub fn merge_discovered(
    discovered: &mut VecDeque<String>,
    new_mints: Vec<String>,
    cap: usize,
) -> usize {
    let mut existing: HashSet<String> = discovered.iter().cloned().collect();
    let mut added = 0;

    for mint in new_mints {
        if existing.insert(mint.clone()) {
            discovered.push_back(mint);
            added += 1;
        }
    }

    while discovered.len() > cap {
        discovered.pop_front();
    }

    added
}
