use crate::config::Config;
use crate::time::now_ts;
use serde::Deserialize;
use std::collections::HashMap;
use std::time::Instant;

// Keep N snapshots per coin to compute velocity
const MAX_SNAPSHOTS: usize = 10;
// Minimum seconds between active-coin polls per coin (prevents DexScreener flooding)
const ACTIVE_POLL_THROTTLE_SECS: u64 = 3;
// Called coins (open positions / dip wait) — poll every second, no throttle
const CALLED_POLL_THROTTLE_SECS: u64 = 1;

#[derive(Debug, Clone)]
pub struct MarketCache {
    pub map: HashMap<String, Vec<MarketSample>>,
    pub last_poll: Instant,
    pub last_active_poll: HashMap<String, Instant>,
}

impl MarketCache {
    pub fn new() -> Self {
        Self {
            map: HashMap::new(),
            last_poll: Instant::now(),
            last_active_poll: HashMap::new(),
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
    // 5m
    pub buys_5m: Option<u64>,
    pub sells_5m: Option<u64>,
    pub volume_5m: Option<f64>,
    // 1h — slow climbers
    pub buys_1h: Option<u64>,
    pub sells_1h: Option<u64>,
    pub volume_1h: Option<f64>,
    // 6h — multi-hour grinders
    pub buys_6h: Option<u64>,
    pub sells_6h: Option<u64>,
    pub volume_6h: Option<f64>,
    // Price change %
    pub price_change_5m: Option<f64>,
    pub price_change_1h: Option<f64>,
    pub price_change_6h: Option<f64>,
}

#[derive(Debug, Clone, Default)]
pub struct MarketTrend {
    pub last_price: Option<f64>,
    pub last_fdv: Option<f64>,
    pub last_liq: Option<f64>,

    // Velocity — % change per minute
    pub fdv_velocity_pct: f64,   // positive = pumping
    pub liq_velocity_pct: f64,

    // Activity — 5m
    pub buys_5m: u64,
    pub sells_5m: u64,
    pub buy_sell_ratio: f64,
    pub volume_5m: f64,
    // Activity — 1h (slow climbers)
    pub buys_1h: u64,
    pub sells_1h: u64,
    pub bsr_1h: f64,
    pub volume_1h: f64,
    // Price change
    pub price_change_5m: f64,
    pub price_change_1h: f64,
    pub price_change_6h: f64,

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

    // SNIPE signals (from psychic-spoon)
    pub early_snipe: bool,         // FDV < $50k AND 15%+ growth in 5m
    pub conviction_momentum: bool, // $15k+ abs FDV gain in 5m on mid cap
    pub fdv_5m_ago: Option<f64>,
    pub fdv_growth_5m_pct: f64,   // % change vs 5m ago
    pub fdv_abs_gain_5m: f64,     // $ change vs 5m ago
    pub late_entry: bool,         // coin peaked 35%+ higher 30m ago — don't chase
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
    t.buys_1h = latest.buys_1h.unwrap_or(0);
    t.sells_1h = latest.sells_1h.unwrap_or(0);
    t.bsr_1h = if t.sells_1h > 0 { t.buys_1h as f64 / t.sells_1h as f64 } else if t.buys_1h > 0 { 5.0 } else { 1.0 };
    t.volume_1h = latest.volume_1h.unwrap_or(0.0);
    t.price_change_5m = latest.price_change_5m.unwrap_or(0.0);
    t.price_change_1h = latest.price_change_1h.unwrap_or(0.0);
    t.price_change_6h = latest.price_change_6h.unwrap_or(0.0);
    t.snapshots = samples.len();

    // ── SNIPE signals (from psychic-spoon) ──────────────────────────
    // Compare current FDV to ~5 min ago snapshot
    let now_ts = latest.ts;
    let fdv_5m_ago = samples.iter()
        .filter(|s| {
            let age = now_ts.saturating_sub(s.ts);
            age >= 240 && age <= 420 // 4-7 min ago window
        })
        .filter_map(|s| s.fdv)
        .next();

    if let Some(fdv_5m) = fdv_5m_ago {
        if fdv_5m > 0.0 {
            let fdv_growth_pct = (fdv - fdv_5m) / fdv_5m;
            let fdv_abs_gain = fdv - fdv_5m;

            // SNIPE: small cap pumping fast
            t.early_snipe = fdv > 0.0
                && fdv < 50_000.0
                && fdv_growth_pct >= 0.15; // 15%+ in 5 min

            // CONVICTION: mid cap with real dollar inflow
            t.conviction_momentum = fdv >= 30_000.0
                && fdv <= 500_000.0
                && fdv_abs_gain >= 15_000.0; // $15k+ absolute gain

            t.fdv_5m_ago = Some(fdv_5m);
            t.fdv_growth_5m_pct = fdv_growth_pct * 100.0;
            t.fdv_abs_gain_5m = fdv_abs_gain;
        }
    }

    // Late entry check — if coin was 35%+ higher 30 min ago, skip
    let fdv_30m_ago = samples.iter()
        .filter(|s| {
            let age = now_ts.saturating_sub(s.ts);
            age >= 1500 && age <= 2100 // 25-35 min ago
        })
        .filter_map(|s| s.fdv)
        .reduce(f64::max);

    if let Some(peak_30m) = fdv_30m_ago {
        t.late_entry = peak_30m > 0.0 && peak_30m >= fdv * 1.35;
    }

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
        // Always poll — caller decides frequency
        let now = now_ts();
        let max_per_cycle = 100usize;

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

    pub async fn poll_active(&mut self, active_mints: &[String]) {
        // Throttle per-coin: at most once every ACTIVE_POLL_THROTTLE_SECS seconds.
        // Prevents DexScreener flooding when main loop runs every 1s with 10 active coins.
        let now_ts_val = now_ts();
        for mint in active_mints {
            let last = self.last_active_poll.get(mint);
            if let Some(t) = last {
                if t.elapsed().as_secs() < ACTIVE_POLL_THROTTLE_SECS {
                    continue; // skip — polled too recently
                }
            }
            if let Ok(sample) = fetch_dex_sample(mint, now_ts_val).await {
                let history = self.map.entry(mint.clone()).or_insert_with(Vec::new);
                history.push(sample);
                if history.len() > MAX_SNAPSHOTS {
                    history.remove(0);
                }
                self.last_active_poll.insert(mint.clone(), Instant::now());
            }
        }
    }

    /// Poll called coins (open positions / dip wait) every second — hawk mode.
    /// No mercy on throttle — these coins have money on the line.
    pub async fn poll_called(&mut self, called_mints: &[String]) {
        let now_ts_val = now_ts();
        for mint in called_mints {
            let last = self.last_active_poll.get(mint);
            if let Some(t) = last {
                if t.elapsed().as_secs() < CALLED_POLL_THROTTLE_SECS {
                    continue;
                }
            }
            if let Ok(sample) = fetch_dex_sample(mint, now_ts_val).await {
                let history = self.map.entry(mint.clone()).or_insert_with(Vec::new);
                history.push(sample);
                if history.len() > MAX_SNAPSHOTS {
                    history.remove(0);
                }
                self.last_active_poll.insert(mint.clone(), Instant::now());
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
    chain_id: Option<String>,
    price_usd: Option<String>,
    fdv: Option<f64>,
    liquidity: Option<DexLiquidity>,
    txns: Option<DexTxns>,
    volume: Option<DexVolume>,
    price_change: Option<DexPriceChange>,
}

#[derive(Debug, Deserialize)]
struct DexLiquidity {
    usd: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct DexTxns {
    m5: Option<DexTxnBucket>,
    h1: Option<DexTxnBucket>,
    h6: Option<DexTxnBucket>,
}

#[derive(Debug, Deserialize)]
struct DexTxnBucket {
    buys: Option<u64>,
    sells: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct DexVolume {
    m5: Option<f64>,
    h1: Option<f64>,
    h6: Option<f64>,
}

#[derive(Debug, Deserialize)]
struct DexPriceChange {
    m5: Option<f64>,
    h1: Option<f64>,
    h6: Option<f64>,
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
        .find(|p| p.chain_id.as_deref() == Some("solana"))
        .or_else(|| pairs.first())
        .ok_or(())?;

    let price = pair.price_usd.as_ref().and_then(|s| s.parse::<f64>().ok());
    let fdv = pair.fdv;
    let liq = pair.liquidity.as_ref().and_then(|l| l.usd);

    // 5m data
    let buys_5m = pair.txns.as_ref().and_then(|t| t.m5.as_ref()).and_then(|m| m.buys);
    let sells_5m = pair.txns.as_ref().and_then(|t| t.m5.as_ref()).and_then(|m| m.sells);
    let volume_5m = pair.volume.as_ref().and_then(|v| v.m5);

    // 1h data — catches slow climbers
    let buys_1h = pair.txns.as_ref().and_then(|t| t.h1.as_ref()).and_then(|m| m.buys);
    let sells_1h = pair.txns.as_ref().and_then(|t| t.h1.as_ref()).and_then(|m| m.sells);
    let volume_1h = pair.volume.as_ref().and_then(|v| v.h1);

    // 6h data — catches multi-hour grinders
    let buys_6h = pair.txns.as_ref().and_then(|t| t.h6.as_ref()).and_then(|m| m.buys);
    let sells_6h = pair.txns.as_ref().and_then(|t| t.h6.as_ref()).and_then(|m| m.sells);
    let volume_6h = pair.volume.as_ref().and_then(|v| v.h6);

    // Price change % over different windows
    let price_change_5m = pair.price_change.as_ref().and_then(|pc| pc.m5);
    let price_change_1h = pair.price_change.as_ref().and_then(|pc| pc.h1);
    let price_change_6h = pair.price_change.as_ref().and_then(|pc| pc.h6);

    Ok(MarketSample {
        ts, price, fdv, liq,
        buys_5m, sells_5m, volume_5m,
        buys_1h, sells_1h, volume_1h,
        buys_6h, sells_6h, volume_6h,
        price_change_5m, price_change_1h, price_change_6h,
    })
}
