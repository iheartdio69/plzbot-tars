use crate::config::Config;
use crate::time::now_ts;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Instant;

// Keep N snapshots per coin to compute velocity
const MAX_SNAPSHOTS: usize = 10;

#[derive(Debug, Clone)]
pub struct MarketCache {
    pub map: HashMap<String, Vec<MarketSample>>,
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
    pub buys_5m: Option<u64>,
    pub sells_5m: Option<u64>,
    pub volume_5m: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct MarketTrend {
    pub last_price: Option<f64>,
    pub last_fdv: Option<f64>,
    pub last_liq: Option<f64>,

    // Velocity — % change per minute
    pub fdv_velocity_pct: f64,   // positive = pumping
    pub liq_velocity_pct: f64,

    // Activity
    pub buys_5m: u64,
    pub sells_5m: u64,
    pub buy_sell_ratio: f64,     // >1.5 = bullish
    pub volume_5m: f64,

    // Flags
    pub price_accel: bool,
    pub fdv_accel: bool,
    pub fdv_ok: bool,
    pub liq_ok: bool,
    pub fdv_over_50k: bool,
    pub fdv_over_150k: bool,
    pub fdv_over_300k: bool,

    // Snapshot count (how much history we have)
    pub snapshots: usize,
}

pub fn market_trend(cache: &MarketCache, mint: &str, cfg: &Config) -> MarketTrend {
    let samples = match cache.map.get(mint) {
        Some(v) if !v.is_empty() => v,
        _ => return MarketTrend::default(),
    };

    let latest = samples.last().unwrap();
    let fdv = latest.fdv.unwrap_or(0.0);
    let liq = latest.liq.unwrap_or(0.0);

    let mut t = MarketTrend::default();
    t.last_price = latest.price;
    t.last_fdv = latest.fdv;
    t.last_liq = latest.liq;
    t.buys_5m = latest.buys_5m.unwrap_or(0);
    t.sells_5m = latest.sells_5m.unwrap_or(0);
    t.volume_5m = latest.volume_5m.unwrap_or(0.0);
    t.snapshots = samples.len();

    // Buy/sell ratio
    let total_tx = t.buys_5m + t.sells_5m;
    t.buy_sell_ratio = if t.sells_5m > 0 {
        t.buys_5m as f64 / t.sells_5m as f64
    } else if t.buys_5m > 0 {
        10.0 // all buys, no sells — very bullish
    } else {
        1.0
    };

    // FDV velocity — compare oldest to latest sample
    if samples.len() >= 2 {
        let oldest = &samples[0];
        let oldest_fdv = oldest.fdv.unwrap_or(0.0);
        let oldest_liq = oldest.liq.unwrap_or(0.0);
        let time_delta_mins = (latest.ts.saturating_sub(oldest.ts)) as f64 / 60.0;

        if time_delta_mins > 0.0 && oldest_fdv > 0.0 {
            let fdv_change_pct = (fdv - oldest_fdv) / oldest_fdv * 100.0;
            t.fdv_velocity_pct = fdv_change_pct / time_delta_mins;
        }
        if time_delta_mins > 0.0 && oldest_liq > 0.0 {
            let liq_change_pct = (liq - oldest_liq) / oldest_liq * 100.0;
            t.liq_velocity_pct = liq_change_pct / time_delta_mins;
        }

        t.fdv_accel = t.fdv_velocity_pct > cfg.fdv_velocity_threshold;
        t.price_accel = t.fdv_accel; // proxy
    }

    t.fdv_ok = fdv >= cfg.min_watch_fdv_usd && fdv <= cfg.max_watch_fdv_usd;
    t.liq_ok = liq >= cfg.min_liq_usd;
    t.fdv_over_50k = fdv >= 50_000.0;
    t.fdv_over_150k = fdv >= 150_000.0;
    t.fdv_over_300k = fdv >= 300_000.0;

    let _ = total_tx; // suppress warning
    t
}

impl MarketCache {
    pub async fn poll(&mut self, cfg: &Config, mints: &[String]) {
        if self.last_poll.elapsed().as_secs() < cfg.market_poll_secs {
            return;
        }
        self.last_poll = Instant::now();

        let max_per_cycle = 50usize;
        let now = now_ts();

        for mint in mints.iter().take(max_per_cycle) {
            if let Ok(sample) = fetch_dex_sample(mint, now).await {
                let history = self.map.entry(mint.clone()).or_insert_with(Vec::new);
                history.push(sample);
                if history.len() > MAX_SNAPSHOTS {
                    history.remove(0);
                }
            }
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DexTokenResp {
    pairs: Option<Vec<DexPairRaw>>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct DexPairRaw {
    price_usd: Option<String>,
    fdv: Option<f64>,
    liquidity: Option<DexLiquidity>,
    txns: Option<DexTxns>,
    volume: Option<DexVolume>,
}

#[derive(Debug, Deserialize)]
struct DexLiquidity {
    usd: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct DexTxns {
    m5: Option<DexTxnBucket>,
}

#[derive(Debug, Deserialize)]
struct DexTxnBucket {
    buys: Option<u64>,
    sells: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct DexVolume {
    m5: Option<f64>,
}

async fn fetch_dex_sample(mint: &str, ts: u64) -> Result<MarketSample, ()> {
    let url = format!("https://api.dexscreener.com/latest/dex/tokens/{}", mint);
    let res = reqwest::get(url).await.map_err(|_| ())?;
    if !res.status().is_success() {
        return Err(());
    }

    let body = res.json::<DexTokenResp>().await.map_err(|_| ())?;

    // Prefer Solana pairs
    let pairs = body.pairs.unwrap_or_default();
    let pair = pairs.iter()
        .find(|_| true) // take first for now — could filter by chainId == "solana"
        .ok_or(())?;

    let price = pair.price_usd.as_ref().and_then(|s| s.parse::<f64>().ok());
    let fdv = pair.fdv;
    let liq = pair.liquidity.as_ref().and_then(|l| l.usd);
    let buys_5m = pair.txns.as_ref().and_then(|t| t.m5.as_ref()).and_then(|m| m.buys);
    let sells_5m = pair.txns.as_ref().and_then(|t| t.m5.as_ref()).and_then(|m| m.sells);
    let volume_5m = pair.volume.as_ref().and_then(|v| v.m5);

    Ok(MarketSample { ts, price, fdv, liq, buys_5m, sells_5m, volume_5m })
}
