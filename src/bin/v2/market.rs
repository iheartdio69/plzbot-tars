use serde::Deserialize;

#[derive(Debug, Clone)]
pub struct DexSnap {
    pub mint: String,
    pub symbol: Option<String>,
    pub name: Option<String>,
    pub price_usd: Option<f64>,
    pub fdv: Option<f64>,
    pub liq_usd: Option<f64>,
    pub tx_5m: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct DexSearchResponse {
    pairs: Option<Vec<DexPair>>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct DexToken {
    address: String,
    name: Option<String>,
    symbol: Option<String>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct DexLiquidity {
    usd: Option<f64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct DexTxnCount {
    buys: Option<u64>,
    sells: Option<u64>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct DexTxns {
    #[serde(rename = "m5")]
    m5: Option<DexTxnCount>,
}

#[derive(Debug, Clone, Deserialize, Default)]
struct DexPair {
    #[serde(rename = "chainId")]
    chain_id: String,
    #[serde(rename = "baseToken")]
    base_token: DexToken,

    #[serde(rename = "priceUsd")]
    price_usd: Option<String>,

    fdv: Option<f64>,
    liquidity: Option<DexLiquidity>,
    txns: Option<DexTxns>,
}

fn parse_price(s: &Option<String>) -> Option<f64> {
    s.as_ref().and_then(|x| x.parse::<f64>().ok())
}

pub async fn fetch_best_snap(client: &reqwest::Client, mint: &str) -> Option<DexSnap> {
    let q_enc = urlencoding::encode(mint);
    let url = format!("https://api.dexscreener.com/latest/dex/search/?q={}", q_enc);

    let res = client.get(&url).send().await.ok()?;
    let body = res.json::<DexSearchResponse>().await.ok()?;

    let pairs = body.pairs.unwrap_or_default();
    if pairs.is_empty() {
        return None;
    }

    let mut best: Option<DexPair> = None;

    for p in pairs.iter() {
        if p.chain_id == "solana" && p.base_token.address == mint {
            best = Some(p.clone());
            break;
        }
    }

    if best.is_none() {
        for p in pairs.iter() {
            if p.chain_id == "solana" {
                best = Some(p.clone());
                break;
            }
        }
    }

    let p = best.unwrap_or_else(|| pairs[0].clone());

    let tx5 = p
        .txns
        .as_ref()
        .and_then(|t| t.m5.as_ref())
        .map(|m| m.buys.unwrap_or(0) + m.sells.unwrap_or(0));

    Some(DexSnap {
        mint: mint.to_string(),
        symbol: p.base_token.symbol.clone(),
        name: p.base_token.name.clone(),
        price_usd: parse_price(&p.price_usd),
        fdv: p.fdv,
        liq_usd: p.liquidity.as_ref().and_then(|l| l.usd),
        tx_5m: tx5,
    })
}
