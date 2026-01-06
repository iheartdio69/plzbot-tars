// market/types.rs (stub for DexPair; expand as needed)
#[derive(Debug, Clone, Deserialize)]
pub struct DexPair {
    pub chain_id: String,
    pub base_token: TokenInfo,
    pub quote_token: TokenInfo,
    pub fdv: Option<f64>,
    pub liquidity: Option<Liquidity>,
    pub txns: Option<Txns>,
    // Add more fields as needed from Dexscreener response
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenInfo {
    pub address: String,
    pub symbol: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Liquidity {
    pub usd: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Txns {
    pub m5: Option<TxnCounts>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TxnCounts {
    pub buys: Option<u64>,
    pub sells: Option<u64>,
}

#[derive(Debug, Default)]
pub struct MarketTrend {
    pub last_fdv: Option<f64>,
    pub last_liq: Option<f64>,
    pub price_accel: f64, // Stub
    pub fdv_accel: f64,   // Stub
}

// market/cache.rs
#[derive(Debug, Default)]
pub struct MarketCache {
    pub map: std::collections::HashMap<String, MarketTrend>,
}

impl MarketCache {
    pub async fn poll(&mut self, cfg: &Config, mints: &[String]) {
        // Stub: Poll Dexscreener for mints, update map
        // Use reqwest to get https://api.dexscreener.com/latest/dex/tokens/{mints.join(",")}
        // Parse, update last_fdv, liq, compute accel if history
    }
}

pub fn market_trend(market: &MarketCache, mint: &str, _cfg: &Config) -> MarketTrend {
    market.map.get(mint).cloned().unwrap_or_default()
}