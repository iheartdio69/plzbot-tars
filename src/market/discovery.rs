// market/discovery.rs
use crate::config::Config;
use std::collections::HashSet;
use std::time::{Duration, Instant};
use reqwest;

#[derive(Debug, Default)]
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
            Some(t) => t.elapsed() >= Duration::from_secs(cfg.market_discovery_every_secs),
        }
    }

    pub async fn run(&mut self, cfg: &Config) -> Vec<String> {
        self.last_run = Some(Instant::now());

        let mut picked: Vec<(String, u64)> = vec![];
        let mut seen: HashSet<String> = HashSet::new();

        for q in &cfg.market_discovery_queries {
            let url = format!("https://api.dexscreener.com/latest/dex/search?q={}", q);
            let resp = reqwest::get(&url).await.ok()?.json::<DexResponse>().await.ok()?;

            for p in resp.pairs {
                if !is_pair_candidate(cfg, &p) {
                    continue;
                }

                let mint = p.base_token.address.clone();
                if seen.insert(mint.clone()) {
                    let tx5m = pair_tx_5m(&p);
                    picked.push((mint, tx5m));
                }
            }
        }

        picked.sort_by_key(|(_, tx)| std::cmp::Reverse(*tx));
        picked.into_iter().take(cfg.market_discovery_top_n).map(|(m, _)| m).collect()
    }
}

#[derive(Debug, Deserialize)]
struct DexResponse {
    pairs: Vec<DexPair>,
}

fn is_pair_candidate(cfg: &Config, p: &DexPair) -> bool {
    if p.chain_id != "solana" {
        return false;
    }
    let base_sym = p.base_token.symbol.as_ref().cloned().unwrap_or_default().to_uppercase();
    let quote_sym = p.quote_token.symbol.as_ref().cloned().unwrap_or_default().to_uppercase();
    if cfg.avoid_bonk && (base_sym.contains("BONK") || quote_sym.contains("BONK")) {
        return false;
    }
    let fdv = p.fdv.unwrap_or(0.0) >= cfg.discovery_min_fdv_usd;
    let liq = p.liquidity.as_ref().map(|l| l.usd).unwrap_or(0.0) >= cfg.discovery_min_liq_usd;
    let tx5m = pair_tx_5m(p) >= cfg.discovery_min_tx_5m;
    fdv && liq && tx5m
}

fn pair_tx_5m(p: &DexPair) -> u64 {
    p.txns.as_ref().and_then(|t| t.m5.as_ref()).map(|m| m.buys.unwrap_or(0) + m.sells.unwrap_or(0)).unwrap_or(0)
}

pub fn merge_discovered(discovered: &mut std::collections::VecDeque<String>, new_mints: Vec<String>, cap: usize) -> usize {
    let mut existing: HashSet<String> = discovered.iter().cloned().collect();
    let mut added = 0;

    for m in new_mints {
        if existing.insert(m.clone()) {
            discovered.push_back(m);
            added += 1;
        }
    }

    while discovered.len() > cap {
        discovered.pop_front();
    }

    added
}