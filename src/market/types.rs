use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DexPair {
    pub chain_id: String,
    pub base_token: TokenInfo,
    pub quote_token: TokenInfo,
    pub fdv: Option<f64>,
    pub liquidity: Option<Liquidity>,
    pub txns: Option<Txns>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TokenInfo {
    pub address: String,
    pub symbol: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Liquidity {
    pub usd: Option<f64>,
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
