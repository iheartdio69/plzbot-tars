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

// Token profiles endpoint — returns newest tokens regardless of name/keyword
#[derive(Deserialize, Debug)]
struct TokenProfile {
    #[serde(rename = "chainId")]
    chain_id: String,
    #[serde(rename = "tokenAddress")]
    token_address: String,
}

#[derive(Default)]
pub struct MarketDiscovery {
    pub last_run: Option<Instant>,
    pub last_profiles_run: Option<Instant>,
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
        let client = reqwest::Client::builder()
            .timeout(Duration::from_secs(10))
            .build()
            .unwrap_or_default();

        // ---------------------------------------------------------------
        // SOURCE 1: DexScreener keyword search (existing)
        // ---------------------------------------------------------------
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

                // age filter — but don't hard-require pairCreatedAt
                // if it's missing, let it through (we'll score it anyway)
                if let Some(cutoff_ms) = min_pair_created_ms {
                    if let Some(created_ms) = pair.pair_created_at {
                        if created_ms < cutoff_ms {
                            continue;
                        }
                    }
                    // if pairCreatedAt missing — allow through, don't skip
                }

                let fdv = match pair.fdv {
                    Some(f) => f,
                    None => continue,
                };

                let liq = pair.liquidity.and_then(|l| l.usd).unwrap_or(0.0);

                let tx_5m = pair
                    .txns
                    .and_then(|t| t.m5)
                    .map(|m| m.buys.unwrap_or(0) + m.sells.unwrap_or(0))
                    .unwrap_or(0);

                let mint = sanitize_mint(pair.base_token.address);

                if mint.is_empty() || !is_probably_pubkey(&mint) {
                    continue;
                }

                if fdv >= cfg.discovery_min_fdv_usd
                    && (liq == 0.0 || liq >= cfg.discovery_min_liq_usd)
                    && (tx_5m as u64) >= cfg.discovery_min_tx_5m
                {
                    found_mints.insert(mint);
                }
            }
        }

        // ---------------------------------------------------------------
        // SOURCE 2: DexScreener token-profiles/latest — newest coins on
        // Solana, no keyword dependency. Runs every discovery cycle.
        // This catches any coin that doesn't contain "pump" in its name.
        // ---------------------------------------------------------------
        let profiles_url = "https://api.dexscreener.com/token-profiles/latest/v1";
        if let Ok(resp) = client.get(profiles_url).send().await {
            if let Ok(profiles) = resp.json::<Vec<TokenProfile>>().await {
                for profile in profiles {
                    if profile.chain_id != "solana" {
                        continue;
                    }
                    let mint = sanitize_mint(profile.token_address);
                    if !mint.is_empty() && is_probably_pubkey(&mint) {
                        found_mints.insert(mint);
                    }
                }
                eprintln!(
                    "DBG discovery: profiles={} keyword_search_total={}",
                    found_mints.len(),
                    found_mints.len()
                );
            }
        }

        // ---------------------------------------------------------------
        // SOURCE 3: DexScreener boosted tokens — often early runners with
        // dev momentum behind them.
        // ---------------------------------------------------------------
        let boosted_url = "https://api.dexscreener.com/token-boosts/latest/v1";
        if let Ok(resp) = client.get(boosted_url).send().await {
            if let Ok(profiles) = resp.json::<Vec<TokenProfile>>().await {
                for profile in profiles {
                    if profile.chain_id != "solana" {
                        continue;
                    }
                    let mint = sanitize_mint(profile.token_address);
                    if !mint.is_empty() && is_probably_pubkey(&mint) {
                        found_mints.insert(mint);
                    }
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
