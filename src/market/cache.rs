use crate::config::Config;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Instant;

#[derive(Debug, Clone)]
pub struct MarketCache {
    pub map: HashMap<String, MarketSample>,
    pub last_poll: Instant,
}

impl MarketCache {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            last_poll: Instant::now(),
        }
    }
}

impl Default for MarketCache {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Default)]
pub struct MarketSample {
    pub ts: u64,
    pub price: Option<f64>,
    pub fdv: Option<f64>,
    pub liq: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct MarketTrend {
    pub last_price: Option<f64>,
    pub last_fdv: Option<f64>,
    pub last_liq: Option<f64>,
    pub price_up: bool,
    pub price_accel: bool,
    pub fdv_accel: bool,
    pub fdv_ok: bool,
    pub liq_ok: bool,
    pub fdv_over_50k: bool,
    pub fdv_over_150k: bool,
    pub fdv_over_300k: bool,
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
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DexLiquidity {
    usd: Option<f64>,
}

pub fn market_trend(cache: &MarketCache, mint: &str, cfg: &Config) -> MarketTrend {
    let s = match cache.map.get(mint) {
        Some(v) => v,
        None => return MarketTrend::default(),
    };

    let fdv = s.fdv;
    let liq = s.liq;

    let mut t = MarketTrend::default();
    t.last_price = s.price;
    t.last_fdv = fdv;
    t.last_liq = liq;

    // NOTE: these depend on Config fields you already have
    t.fdv_ok =
        fdv.unwrap_or(0.0) >= cfg.min_watch_fdv_usd && fdv.unwrap_or(0.0) <= cfg.max_watch_fdv_usd;
    t.liq_ok = liq.unwrap_or(0.0) >= cfg.min_liq_usd;

    let f = fdv.unwrap_or(0.0);
    t.fdv_over_50k = f >= 50_000.0;
    t.fdv_over_150k = f >= 150_000.0;
    t.fdv_over_300k = f >= 300_000.0;

    // no history in this minimal cache -> accel false
    t
}

impl MarketCache {
    pub async fn poll(&mut self, cfg: &Config, mints: &[String]) {
        // throttle: only poll every cfg.market_poll_secs
        if self.last_poll.elapsed().as_secs() < cfg.market_poll_secs {
            return;
        }
        self.last_poll = Instant::now();

        // poll at most N per cycle (protect yourself)
        let max_per_cycle = 50usize;
        let now_ts = crate::time::now();

        for mint in mints.iter().take(max_per_cycle) {
            if let Ok(sample) = fetch_dex_sample(mint, now_ts).await {
                self.map.insert(mint.clone(), sample);
            }
        }
    }
}

async fn fetch_dex_sample(mint: &str, ts: u64) -> Result<MarketSample, ()> {
    // Dexscreener token endpoint
    let url = format!("https://api.dexscreener.com/latest/dex/tokens/{}", mint);
    let res = reqwest::get(url).await.map_err(|_| ())?;
    if !res.status().is_success() {
        return Err(());
    }

    let body = res.json::<DexTokenResp>().await.map_err(|_| ())?;
    let pair = body.pairs.and_then(|mut p| p.pop()).ok_or(())?;

    let price = pair.price_usd.and_then(|s| s.parse::<f64>().ok());
    let fdv = pair.fdv;
    let liq = pair.liquidity.and_then(|l| l.usd);

    Ok(MarketSample {
        ts,
        price,
        fdv,
        liq,
    })
}
