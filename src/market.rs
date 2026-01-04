use crate::config::{MAX_FDV_USD, MIN_LIQ_USD};
use crate::time::now;
use crate::types::{MarketCache, MarketSnap, MarketTrend};
use reqwest::Client;
use serde::Deserialize;
use std::collections::VecDeque;

pub fn cache_push(cache: &mut MarketCache, mint: &str, snap: MarketSnap) {
    let q = cache.entry(mint.to_string()).or_insert_with(VecDeque::new);
    q.push_back(snap);
    while q.len() > 30 {
        q.pop_front();
    }
}

pub fn market_trend(cache: &MarketCache, mint: &str) -> MarketTrend {
    let Some(q) = cache.get(mint) else { return MarketTrend::default(); };
    if q.len() < 2 { return MarketTrend::default(); }

    let first = &q[0];
    let last = &q[q.len() - 1];

    let price_up = match (first.price_usd, last.price_usd) {
        (Some(a), Some(b)) if a > 0.0 && b > a => true,
        _ => false,
    };

    let fdv_ok = match last.fdv {
        Some(fdv) => fdv > 0.0 && fdv <= MAX_FDV_USD,
        None => false,
    };

    let liq_ok = match last.liquidity_usd {
        Some(l) => l >= MIN_LIQ_USD,
        None => false,
    };

    MarketTrend {
        price_up,
        fdv_ok,
        liq_ok,
        last_price: last.price_usd,
        last_fdv: last.fdv,
        last_liq: last.liquidity_usd,
    }
}

#[derive(Debug, Deserialize)]
struct DexSearchResp { pairs: Vec<DexPair> }

#[derive(Debug, Deserialize)]
struct DexPair {
    chainId: Option<String>,
    priceUsd: Option<String>,
    fdv: Option<f64>,
    liquidity: Option<DexLiquidity>,
    volume: Option<DexVolume>,
    baseToken: Option<DexToken>,
}

#[derive(Debug, Deserialize)]
struct DexLiquidity { usd: Option<f64> }

#[derive(Debug, Deserialize)]
struct DexVolume { h24: Option<f64> }

#[derive(Debug, Deserialize)]
struct DexToken { address: Option<String> }

pub async fn fetch_dexscreener_snap(client: &Client, mint: &str) -> Option<MarketSnap> {
    let url = format!("https://api.dexscreener.com/latest/dex/search?q={}", mint);
    let res = client.get(url).send().await.ok()?;
    let parsed = res.json::<DexSearchResp>().await.ok()?;

    let mut best: Option<&DexPair> = None;
    for p in parsed.pairs.iter() {
        let chain_ok = p.chainId.as_deref().unwrap_or("").eq_ignore_ascii_case("solana");
        if !chain_ok { continue; }
        let base_addr = p.baseToken.as_ref().and_then(|t| t.address.as_deref()).unwrap_or("");
        if base_addr != mint { continue; }
        best = Some(p);
        break;
    }

    let p = best?;
    let price_usd = p.priceUsd.as_ref().and_then(|s| s.parse::<f64>().ok());
    let liquidity_usd = p.liquidity.as_ref().and_then(|l| l.usd);
    let vol_h24 = p.volume.as_ref().and_then(|v| v.h24);

    Some(MarketSnap {
        ts: now(),
        price_usd,
        fdv: p.fdv,
        liquidity_usd,
        vol_h24,
    })
}