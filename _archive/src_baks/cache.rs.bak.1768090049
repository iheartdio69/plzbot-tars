use crate::config::Config;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct MarketCache {
    pub map: HashMap<String, MarketSample>,
    pub last_poll: Instant,
    pub cursor: usize,
}

impl Default for MarketCache {
    fn default() -> Self {
        Self {
            map: HashMap::new(),
            last_poll: Instant::now(),
            cursor: 0,
        }
    }
}

#[derive(Debug, Clone, Default)]
pub struct MarketSample {
    pub ts: u64,
    pub price: Option<f64>,
    pub fdv: Option<f64>,
    pub liq: Option<f64>,
    pub tx_5m: Option<u64>,
    pub buys_5m: Option<u64>,
    pub sells_5m: Option<u64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DexTokenResp {
    pairs: Option<Vec<DexPair>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DexPair {
    price_usd: Option<String>,
    fdv: Option<f64>,
    liquidity: Option<DexLiquidity>,
    txns: Option<DexTxns>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DexLiquidity {
    usd: Option<f64>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DexTxns {
    m5: Option<DexTxnPeriod>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DexTxnPeriod {
    buys: Option<u64>,
    sells: Option<u64>,
}

impl MarketCache {
    pub async fn poll(&mut self, _cfg: &Config, mints: &[String]) {
        let max_per_cycle = 25usize;
        let now_ts = crate::time::now();

        if mints.is_empty() {
            self.last_poll = Instant::now();
            return;
        }

        let n = mints.len();
        let start = self.cursor % n;

        for i in 0..std::cmp::min(max_per_cycle, n) {
            let idx = (start + i) % n;
            let mint = &mints[idx];
            if let Ok(sample) = fetch_dex_sample(mint, now_ts).await {
                self.map.insert(mint.clone(), sample);
            }
        }

        self.cursor = (start + max_per_cycle) % n;
        self.last_poll = Instant::now();
    }
}

async fn fetch_dex_sample(mint: &str, ts: u64) -> Result<MarketSample, ()> {
    let url = format!("https://api.dexscreener.com/latest/dex/tokens/{}", mint);
    let res = reqwest::get(url).await.map_err(|_| ())?;
    if !res.status().is_success() {
        return Err(());
    }

    let body = res.json::<DexTokenResp>().await.map_err(|_| ())?;
    let mut pairs = body.pairs.ok_or(())?;

    eprintln!("DBG dex pairs_len={}", pairs.len());
    for (i, p) in pairs.iter().take(3).enumerate() {
        let lu = p.liquidity.as_ref().and_then(|l| l.usd);
        eprintln!("DBG dex pair[{}] fdv={:?} liq_usd={:?}", i, p.fdv, lu);
    }

    pairs.sort_by(|a, b| {
        let la = a.liquidity.as_ref().and_then(|l| l.usd).unwrap_or(0.0);
        let lb = b.liquidity.as_ref().and_then(|l| l.usd).unwrap_or(0.0);
        lb.partial_cmp(&la).unwrap_or(std::cmp::Ordering::Equal)
    });

    let pair = pairs.into_iter().next().ok_or(())?;

    eprintln!(
        "DBG dex pair fdv={:?} liq_field_present={} liq_usd={:?}",
        pair.fdv,
        pair.liquidity.is_some(),
        pair.liquidity.as_ref().and_then(|l| l.usd)
    );

    let price = pair.price_usd.and_then(|s| s.parse::<f64>().ok());
    let fdv = pair.fdv;
    let liq = pair.liquidity.and_then(|l| l.usd);

    let (buys_5m, sells_5m, tx_5m) = match pair.txns.and_then(|t| t.m5) {
        Some(p) => {
            let b = p.buys.unwrap_or(0);
            let s = p.sells.unwrap_or(0);
            (Some(b), Some(s), Some(b + s))
        }
        None => (None, None, None),
    };

    Ok(MarketSample {
        ts,
        price,
        fdv,
        liq,
        tx_5m,
        buys_5m,
        sells_5m,
    })
}
