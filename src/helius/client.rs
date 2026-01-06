use crate::helius::types::HeliusTx;

pub async fn fetch_address_txs(
    helius_addr_url: &str,
    api_key: &str,
    address: &str,
    limit: usize,
) -> Result<Vec<HeliusTx>, ()> {
    let url = format!(
        "{}/{}/transactions?api-key={}&limit={}",
        helius_addr_url.trim_end_matches('/'),
        address,
        api_key,
        limit
    );

    let res = reqwest::get(url).await.map_err(|_| ())?;
    if !res.status().is_success() {
        return Err(());
    }
    res.json::<Vec<HeliusTx>>().await.map_err(|_| ())
}
