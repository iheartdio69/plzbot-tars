use anyhow::Result;
use reqwest::Client;
use solana_sdk::signature::Signer;

fn load_keypair(private_key: &str) -> Result<solana_sdk::signature::Keypair> {
    let key = private_key.trim();
    // Base58 encoded — standard Solana CLI format (64 bytes: secret + public)
    if let Ok(bytes) = bs58::decode(key).into_vec() {
        if bytes.len() >= 32 {
            return solana_sdk::signature::keypair_from_seed(&bytes[..32])
                .map_err(|e| anyhow::anyhow!("keypair_from_seed: {}", e));
        }
    }
    // JSON array format (Phantom / solana-keygen file)
    if let Ok(bytes) = serde_json::from_str::<Vec<u8>>(key) {
        if bytes.len() >= 32 {
            return solana_sdk::signature::keypair_from_seed(&bytes[..32])
                .map_err(|e| anyhow::anyhow!("keypair_from_seed: {}", e));
        }
    }
    Err(anyhow::anyhow!("Invalid private key format"))
}

pub async fn buy(
    public_key: &str,
    private_key: &str,
    token_mint: &str,
    sol_amount: f64,
    rpc_url: &str,
) -> Result<String> {
    if private_key.trim().is_empty() {
        return Err(anyhow::anyhow!("PUMPPORTAL_PRIVATE_KEY not set"));
    }

    let client = Client::new();

    let body = serde_json::json!({
        "publicKey": public_key,
        "action": "buy",
        "mint": token_mint,
        "amount": sol_amount,
        "denominatedInSol": "true",
        "slippage": 15,
        "priorityFee": 0.00005,
        "pool": "auto"
    });

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        client
            .post("https://pumpportal.fun/api/trade-local")
            .json(&body)
            .send()
    ).await??;

    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("PumpPortal API error: {}", resp.status()));
    }

    let tx_bytes = resp.bytes().await?;

    let keypair = load_keypair(private_key)?;
    let mut tx: solana_sdk::transaction::VersionedTransaction =
        bincode::deserialize(&tx_bytes)?;
    let message_bytes = tx.message.serialize();
    let sig = keypair.sign_message(&message_bytes);
    tx.signatures[0] = sig;

    let tx_b64 = base64::encode(bincode::serialize(&tx)?);
    let rpc_resp: serde_json::Value = client
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

    if let Some(err) = rpc_resp.get("error") {
        return Err(anyhow::anyhow!("RPC error: {}", err));
    }

    let signature = rpc_resp["result"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    eprintln!("🤖 TARS BUY executed: mint={} sol={} sig={}",
        &token_mint[..8], sol_amount, &signature[..8]);

    Ok(signature)
}

pub async fn sell_percent(
    public_key: &str,
    private_key: &str,
    token_mint: &str,
    percent: f64,
    rpc_url: &str,
) -> Result<String> {
    if private_key.trim().is_empty() {
        return Err(anyhow::anyhow!("PUMPPORTAL_PRIVATE_KEY not set"));
    }

    let client = Client::new();
    let amount = format!("{}%", percent as u64);

    let body = serde_json::json!({
        "publicKey": public_key,
        "action": "sell",
        "mint": token_mint,
        "amount": amount,
        "denominatedInSol": "false",
        "slippage": 15,
        "priorityFee": 0.00005,
        "pool": "auto"
    });

    let resp = tokio::time::timeout(
        std::time::Duration::from_secs(10),
        client
            .post("https://pumpportal.fun/api/trade-local")
            .json(&body)
            .send()
    ).await??;

    if !resp.status().is_success() {
        return Err(anyhow::anyhow!("PumpPortal sell error: {}", resp.status()));
    }

    let tx_bytes = resp.bytes().await?;
    let keypair = load_keypair(private_key)?;
    let mut tx: solana_sdk::transaction::VersionedTransaction =
        bincode::deserialize(&tx_bytes)?;
    let message_bytes = tx.message.serialize();
    let sig = keypair.sign_message(&message_bytes);
    tx.signatures[0] = sig;

    let tx_b64 = base64::encode(bincode::serialize(&tx)?);
    let rpc_resp: serde_json::Value = client
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

    if let Some(err) = rpc_resp.get("error") {
        return Err(anyhow::anyhow!("RPC error: {}", err));
    }

    let signature = rpc_resp["result"]
        .as_str()
        .unwrap_or("unknown")
        .to_string();

    eprintln!("🤖 TARS SELL executed: mint={} pct={}% sig={}",
        &token_mint[..8], percent, &signature[..8]);

    Ok(signature)
}

#[derive(Debug, Clone)]
pub struct Position {
    pub mint: String,
    pub entry_fdv: f64,
    pub sol_spent: f64,
    pub opened_ts: i64,
    pub tp1_hit: bool,
    pub tp2_hit: bool,
    pub tp3_hit: bool,
    pub tp2_hit_ts: i64,
    pub closed: bool,
}

impl Position {
    pub fn new(mint: &str, entry_fdv: f64, sol_spent: f64, now_ts: i64) -> Self {
        Self {
            mint: mint.to_string(),
            entry_fdv,
            sol_spent,
            opened_ts: now_ts,
            tp1_hit: false,
            tp2_hit: false,
            tp3_hit: false,
            tp2_hit_ts: 0,
            closed: false,
        }
    }

    pub fn check_exits(
        &mut self,
        current_fdv: f64,
        tp1_mult: f64,
        tp2_mult: f64,
        sl_pct: f64,
        now_ts: i64,
    ) -> Option<ExitSignal> {
        if self.closed {
            return None;
        }

        let mult = current_fdv / self.entry_fdv;

        // Stop loss — sell 95%, keep 5% moon bag forever
        if mult <= (1.0 - sl_pct) {
            self.closed = true;
            return Some(ExitSignal::StopLoss);
        }

        // TP3 — hit 3x, sell 15%, keep 5% moon bag
        if !self.tp3_hit && mult >= 3.0 {
            self.tp3_hit = true;
            return Some(ExitSignal::TakeProfit3);
        }

        // Post-TP2 exit logic — smart exit if not heading to 3x
        if self.tp2_hit && !self.tp3_hit && self.tp2_hit_ts > 0 {
            let time_since_tp2 = now_ts - self.tp2_hit_ts;

            // Real breakdown — fell well below 2x, not just a fib dip
            let real_breakdown = mult < 1.75;

            // Timed out — 20 minutes since TP2 with no 3x
            let timed_out = time_since_tp2 > 1200;

            if real_breakdown || timed_out {
                self.tp3_hit = true;
                return Some(ExitSignal::TakeProfit3);
            }
        }

        // TP2 at 2x — sell 30%, start the clock
        if !self.tp2_hit && mult >= tp2_mult {
            self.tp2_hit = true;
            self.tp2_hit_ts = now_ts;
            return Some(ExitSignal::TakeProfit2);
        }

        // TP1 at 1.5x — sell 50%
        if !self.tp1_hit && mult >= tp1_mult {
            self.tp1_hit = true;
            return Some(ExitSignal::TakeProfit1);
        }

        None
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum ExitSignal {
    TakeProfit1,  // sell 50%
    TakeProfit2,  // sell 30%
    TakeProfit3,  // sell 15% — then 5% moon bag stays forever
    StopLoss,     // sell 95% — keep 5% moon bag forever
}
