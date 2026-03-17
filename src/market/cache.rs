use crate::config::Config;
use crate::market::dexscreener;
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
    pub pair_address: Option<String>,
}

impl MarketCache {
    pub async fn poll(&mut self, _cfg: &Config, priority: &[String], mints: &[String]) {
        let now_ts = crate::time::now();

        // 1) always refresh priority first
        let prio_cap = 50usize;
        for mint in priority.iter().take(prio_cap) {
            if let Some(sample) = fetch_dex_sample(mint, now_ts).await {
                self.map.insert(mint.clone(), sample);
            }
        }

        // 2) then round-robin the rest
        let max_per_cycle = 75usize; // round-robin budget
        if mints.is_empty() {
            self.last_poll = Instant::now();
            return;
        }

        let n = mints.len();
        let start = self.cursor % n;

        for i in 0..std::cmp::min(max_per_cycle, n) {
            let idx = (start + i) % n;
            let mint = &mints[idx];

            if let Some(sample) = fetch_dex_sample(mint, now_ts).await {
                self.map.insert(mint.clone(), sample);
            }
        }

        self.cursor = (start + max_per_cycle) % n;
        self.last_poll = Instant::now();
    }
}

async fn fetch_dex_sample(mint: &str, ts: u64) -> Option<MarketSample> {
    let snap = dexscreener::fetch_dexscreener_snap(mint).await?;

    let price = snap.price_usd.and_then(|s| s.parse::<f64>().ok());
    let fdv = snap.fdv;
    let liq = snap.liquidity.as_ref().and_then(|l| l.usd);

    let (buys_5m, sells_5m, tx_5m) = match snap.txns.and_then(|t| t.m5) {
        Some(p) => {
            let b = p.buys.unwrap_or(0);
            let s = p.sells.unwrap_or(0);
            (Some(b), Some(s), Some(b + s))
        }
        None => (None, None, None),
    };

    Some(MarketSample {
        ts,
        price,
        fdv,
        liq,
        tx_5m,
        buys_5m,
        sells_5m,
        pair_address: Some(snap.pair_address),
    })
}
