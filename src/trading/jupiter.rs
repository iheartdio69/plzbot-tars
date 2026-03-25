/// Jupiter swap executor — handles bonded/graduated coins that PumpPortal can't
/// Used as fallback when PumpPortal returns 400 (coin graduated to Raydium)

use anyhow::Result;
use reqwest::Client;
use solana_sdk::signature::Signer;

const JUPITER_QUOTE: &str = "https://quote-api.jup.ag/v6/quote";
const JUPITER_SWAP:  &str = "https://quote-api.jup.ag/v6/swap";
const SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const LAMPORTS: u64 = 1_000_000_000;

fn load_keypair(private_key: &str) -> Result<solana_sdk::signature::Keypair> {
    let key = private_key.trim();
    if let Ok(bytes) = bs58::decode(key).into_vec() {
        if bytes.len() >= 32 {
            return solana_sdk::signature::keypair_from_seed(&bytes[..32])
                .map_err(|e| anyhow::anyhow!("{}", e));
        }
    }
    Err(anyhow::anyhow!("Invalid private key"))
}

fn b64(data: &[u8]) -> String {
    const C: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut o = String::new();
    let mut i = 0;
    while i < data.len() {
        let b0 = data[i] as usize;
        let b1 = if i+1 < data.len() { data[i+1] as usize } else { 0 };
        let b2 = if i+2 < data.len() { data[i+2] as usize } else { 0 };
        o.push(C[(b0>>2)&0x3f] as char);
        o.push(C[((b0&3)<<4)|((b1>>4)&0xf)] as char);
        o.push(if i+1<data.len(){C[((b1&0xf)<<2)|((b2>>6)&3)]as char}else{'='});
        o.push(if i+2<data.len(){C[b2&0x3f]as char}else{'='});
        i += 3;
    }
    o
}

/// Buy `sol_amount` SOL worth of `token_mint` via Jupiter
pub async fn buy(
    pubkey: &str,
    private_key: &str,
    token_mint: &str,
    sol_amount: f64,
    rpc_url: &str,
) -> Result<String> {
    let client = Client::new();
    let lamports = (sol_amount * LAMPORTS as f64) as u64;

    // Get quote
    let quote: serde_json::Value = client
        .get(format!(
            "{}?inputMint={}&outputMint={}&amount={}&slippageBps=500&dynamicSlippage=true",
            JUPITER_QUOTE, SOL_MINT, token_mint, lamports
        ))
        .send().await?
        .json().await
        .map_err(|e| anyhow::anyhow!("Jupiter quote: {}", e))?;

    if quote.get("error").is_some() {
        return Err(anyhow::anyhow!("Jupiter quote error: {}", quote["error"]));
    }

    // Get swap transaction
    let swap_resp: serde_json::Value = client
        .post(JUPITER_SWAP)
        .json(&serde_json::json!({
            "quoteResponse": quote,
            "userPublicKey": pubkey,
            "wrapAndUnwrapSol": true,
            "dynamicComputeUnitLimit": true,
            "prioritizationFeeLamports": "auto"
        }))
        .send().await?
        .json().await?;

    let swap_tx = swap_resp["swapTransaction"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No swapTransaction in Jupiter response"))?;

    // Decode base64 tx
    let tx_bytes = {
        let s = swap_tx;
        let mut out = Vec::new();
        let chars: Vec<u8> = s.bytes().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = |b: u8| -> u8 {
                match b {
                    b'A'..=b'Z' => b - b'A',
                    b'a'..=b'z' => b - b'a' + 26,
                    b'0'..=b'9' => b - b'0' + 52,
                    b'+' => 62,
                    b'/' => 63,
                    _ => 0,
                }
            };
            let b0 = chars[i]; let b1 = if i+1<chars.len(){chars[i+1]}else{b'='};
            let b2 = if i+2<chars.len(){chars[i+2]}else{b'='};
            let b3 = if i+3<chars.len(){chars[i+3]}else{b'='};
            out.push((c(b0)<<2)|(c(b1)>>4));
            if b2!=b'=' { out.push(((c(b1)&0xf)<<4)|(c(b2)>>2)); }
            if b3!=b'=' { out.push(((c(b2)&3)<<6)|c(b3)); }
            i += 4;
        }
        out
    };

    // Sign and send
    let keypair = load_keypair(private_key)?;
    let mut tx: solana_sdk::transaction::VersionedTransaction = bincode::deserialize(&tx_bytes)?;
    let message_bytes = tx.message.serialize();
    let sig = keypair.sign_message(&message_bytes);
    tx.signatures[0] = sig;

    let tx_b64 = b64(&bincode::serialize(&tx)?);
    let rpc_resp: serde_json::Value = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "sendTransaction",
            "params": [tx_b64, {"encoding": "base64", "skipPreflight": true}]
        }))
        .send().await?.json().await?;

    if let Some(err) = rpc_resp.get("error") {
        return Err(anyhow::anyhow!("RPC error: {}", err));
    }

    let signature = rpc_resp["result"].as_str().unwrap_or("unknown").to_string();
    println!("  🪐 JUP BUY {} {} SOL sig:{}", &token_mint[..8], sol_amount, &signature[..8]);
    Ok(signature)
}

/// Sell `percent`% of token via Jupiter
pub async fn sell(
    pubkey: &str,
    private_key: &str,
    token_mint: &str,
    percent: f64,
    rpc_url: &str,
) -> Result<String> {
    let client = Client::new();

    // Get token balance first
    let bal_resp: serde_json::Value = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0", "id": 1,
            "method": "getTokenAccountsByOwner",
            "params": [pubkey,
                {"mint": token_mint},
                {"encoding": "jsonParsed"}
            ]
        }))
        .send().await?.json().await?;

    let amount_str = bal_resp["result"]["value"]
        .as_array()
        .and_then(|a| a.first())
        .and_then(|v| v["account"]["data"]["parsed"]["info"]["tokenAmount"]["amount"].as_str())
        .unwrap_or("0");

    let total: u64 = amount_str.parse().unwrap_or(0);
    if total == 0 {
        return Err(anyhow::anyhow!("No tokens to sell"));
    }

    let sell_amount = ((total as f64) * percent / 100.0) as u64;

    let quote: serde_json::Value = client
        .get(format!(
            "{}?inputMint={}&outputMint={}&amount={}&slippageBps=500",
            JUPITER_QUOTE, token_mint, SOL_MINT, sell_amount
        ))
        .send().await?.json().await?;

    let swap_resp: serde_json::Value = client
        .post(JUPITER_SWAP)
        .json(&serde_json::json!({
            "quoteResponse": quote,
            "userPublicKey": pubkey,
            "wrapAndUnwrapSol": true,
            "dynamicComputeUnitLimit": true,
            "prioritizationFeeLamports": "auto"
        }))
        .send().await?.json().await?;

    let swap_tx = swap_resp["swapTransaction"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("No swapTransaction"))?;

    // Same decode/sign/send as buy
    let tx_bytes = {
        let s = swap_tx;
        let mut out = Vec::new();
        let chars: Vec<u8> = s.bytes().collect();
        let mut i = 0;
        while i < chars.len() {
            let c = |b: u8| -> u8 { match b { b'A'..=b'Z'=>b-b'A', b'a'..=b'z'=>b-b'a'+26, b'0'..=b'9'=>b-b'0'+52, b'+'=>62, b'/'=>63, _=>0 } };
            let b0=chars[i]; let b1=if i+1<chars.len(){chars[i+1]}else{b'='};
            let b2=if i+2<chars.len(){chars[i+2]}else{b'='};
            let b3=if i+3<chars.len(){chars[i+3]}else{b'='};
            out.push((c(b0)<<2)|(c(b1)>>4));
            if b2!=b'=' { out.push(((c(b1)&0xf)<<4)|(c(b2)>>2)); }
            if b3!=b'=' { out.push(((c(b2)&3)<<6)|c(b3)); }
            i += 4;
        }
        out
    };

    let keypair = load_keypair(private_key)?;
    let mut tx: solana_sdk::transaction::VersionedTransaction = bincode::deserialize(&tx_bytes)?;
    let sig = keypair.sign_message(&tx.message.serialize());
    tx.signatures[0] = sig;
    let tx_b64 = b64(&bincode::serialize(&tx)?);

    let rpc_resp: serde_json::Value = client
        .post(rpc_url)
        .json(&serde_json::json!({"jsonrpc":"2.0","id":1,"method":"sendTransaction",
            "params":[tx_b64,{"encoding":"base64","skipPreflight":true}]}))
        .send().await?.json().await?;

    if let Some(err) = rpc_resp.get("error") {
        return Err(anyhow::anyhow!("RPC error: {}", err));
    }

    Ok(rpc_resp["result"].as_str().unwrap_or("unknown").to_string())
}
