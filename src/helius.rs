use crate::config::{FETCH_LIMIT, HELIUS_ADDR_URL, HELIUS_API_KEY, PUMP_FUN_PROGRAM};
use crate::types::HeliusTx;
use reqwest::Client;

pub async fn fetch_latest_program_txs(client: &Client) -> Result<Vec<HeliusTx>, ()> {
    let url = format!(
        "{}/{}/transactions?api-key={}&limit={}",
        HELIUS_ADDR_URL,
        PUMP_FUN_PROGRAM,
        HELIUS_API_KEY,
        FETCH_LIMIT
    );

    let res = client.get(url).send().await.map_err(|_| ())?;
    res.json::<Vec<HeliusTx>>().await.map_err(|_| ())
}