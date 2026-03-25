/// PumpPortal trade executor
/// Ported from psychic-spoon/tars.rs — battle-tested
/// Uses https://pumpportal.fun/api/trade-local for instant execution
/// Works for pump.fun bonding curve + Raydium + auto pool detection

use anyhow::Result;
use reqwest::Client;
use solana_sdk::signature::Signer;

const PUMPPORTAL_URL: &str = "https://pumpportal.fun/api/trade-local";

fn load_keypair(private_key: &str) -> Result<solana_sdk::signature::Keypair> {
    let key = private_key.trim();
    // Base58 encoded — standard Solana CLI format
    if let Ok(bytes) = bs58::decode(key).into_vec() {
        if bytes.len() >= 32 {
            return solana_sdk::signature::keypair_from_seed(&bytes[..32])
                .map_err(|e| anyhow::anyhow!("keypair_from_seed: {}", e));
        }
    }
    // JSON array format (Phantom / solana-keygen)
    if let Ok(bytes) = serde_json::from_str::<Vec<u8>>(key) {
        if bytes.len() >= 32 {
            return solana_sdk::signature::keypair_from_seed(&bytes[..32])
                .map_err(|e| anyhow::anyhow!("keypair_from_seed: {}", e));
        }
    }
    Err(anyhow::anyhow!("Invalid private key format"))
}

async fn send_tx(tx_bytes: &[u8], private_key: &str, rpc_url: &str) -> Result<String> {
    let keypair = load_keypair(private_key)?;
    let mut tx: solana_sdk::transaction::VersionedTransaction =
        bincode::deserialize(tx_bytes)?;
    let message_bytes = tx.message.serialize();
    let sig = keypair.sign_message(&message_bytes);
    tx.signatures[0] = sig;

    let tx_bytes_ser = bincode::serialize(&tx)?;
    // Encode as base64 using STANDARD alphabet
    let tx_b64 = {
        fn b64(input: &[u8]) -> String {
            const C: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
            let mut o = String::new();
            let mut i = 0;
            while i < input.len() {
                let b0 = input[i] as usize;
                let b1 = if i+1 < input.len() { input[i+1] as usize } else { 0 };
                let b2 = if i+2 < input.len() { input[i+2] as usize } else { 0 };
                o.push(C[(b0>>2)&0x3f] as char);
                o.push(C[((b0&3)<<4)|((b1>>4)&0xf)] as char);
                o.push(if i+1<input.len(){C[((b1&0xf)<<2)|((b2>>6)&3)]as char}else{'='});
                o.push(if i+2<input.len(){C[b2&0x3f]as char}else{'='});
                i += 3;
            }
            o
        }
        b64(&tx_bytes_ser)
    };
    let client = Client::new();
    let resp: serde_json::Value = client
        .post(rpc_url)
        .json(&serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "sendTransaction",
            "params": [tx_b64, {"encoding": "base64", "skipPreflight": true}]
        }))
        .send()
        .await?
        .json()
        .await?;

    if let Some(err) = resp.get("error") {
        return Err(anyhow::anyhow!("RPC error: {}", err));
    }

    Ok(resp["result"].as_str().unwrap_or("unknown").to_string())
}

/// Buy `sol_amount` SOL worth of `token_mint`
/// Tries PumpPortal first (lower fees for bonding curve coins)
/// Falls back to Jupiter automatically for graduated/Raydium coins
pub async fn buy(
    public_key: &str,
    private_key: &str,
    api_key: &str,
    token_mint: &str,
    sol_amount: f64,
    rpc_url: &str,
) -> Result<String> {
    let client = Client::new();
    let mut body = serde_json::json!({
        "publicKey": public_key,
        "action": "buy",
        "mint": token_mint,
        "amount": sol_amount,
        "denominatedInSol": "true",
        "slippage": 25,
        "priorityFee": 0.0002,
        "pool": "pump"
    });

    if !api_key.is_empty() {
        body["apiKey"] = serde_json::json!(api_key);
    }

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        client.post(PUMPPORTAL_URL).json(&body).send()
    ).await??;

    if resp.status().is_success() {
        let tx_bytes = resp.bytes().await?;
        let signature = send_tx(&tx_bytes, private_key, rpc_url).await?;
        println!("  🚀 PP BUY {} {} SOL sig:{}", &token_mint[..8], sol_amount, &signature[..8]);
        return Ok(signature);
    }

    // PumpPortal failed (400 = coin graduated to Raydium) — fall back to Jupiter
    println!("  ↪️  PP failed ({}), trying Jupiter...", resp.status());
    let sig = crate::trading::jupiter::buy(public_key, private_key, token_mint, sol_amount, rpc_url).await?;
    Ok(sig)
}

/// Sell `percent`% of held tokens for `token_mint`
/// Keep 5% moon bag (pass 95 for full exit, 50 for half, etc.)
pub async fn sell(
    public_key: &str,
    private_key: &str,
    api_key: &str,
    token_mint: &str,
    percent: f64,
    rpc_url: &str,
) -> Result<String> {
    let client = Client::new();
    let amount = format!("{}%", percent as u64);

    let mut body = serde_json::json!({
        "publicKey": public_key,
        "action": "sell",
        "mint": token_mint,
        "amount": amount,
        "denominatedInSol": "false",
        "slippage": 15,
        "priorityFee": 0.00005,
        "pool": "pump"
    });

    if !api_key.is_empty() {
        body["apiKey"] = serde_json::json!(api_key);
    }

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        client.post(PUMPPORTAL_URL).json(&body).send()
    ).await??;

    if resp.status().is_success() {
        let tx_bytes = resp.bytes().await?;
        let signature = send_tx(&tx_bytes, private_key, rpc_url).await?;
        println!("  💰 PP SELL {}% {} sig:{}", percent, &token_mint[..8], &signature[..8]);
        return Ok(signature);
    }

    // Fall back to Jupiter for graduated coins
    println!("  ↪️  PP sell failed, trying Jupiter...");
    let sig = crate::trading::jupiter::sell(public_key, private_key, token_mint, percent, rpc_url).await?;
    Ok(sig)
}
