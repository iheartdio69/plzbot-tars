// helius/websocket.rs
// Subscribes to pump.fun program logs via Helius WebSocket.
// Catches new token mints the SECOND they happen — way before DexScreener discovery.
// Runs as a background task, pushing new mints into a shared sink.

use crate::config::Config;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tokio::time::{sleep, Duration};
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

pub type NewMintsSink = Arc<Mutex<Vec<String>>>;

const PUMP_FUN_PROGRAM: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";

pub async fn subscribe_pump_fun(cfg: &Config, sink: NewMintsSink) {
    let mut total_seen = 0u64;
    loop {
        println!("🔌 Helius WebSocket connecting...");
        match listen_once(cfg, sink.clone(), &mut total_seen).await {
            Ok(_) => println!("⚠️  WebSocket disconnected — reconnecting in 3s"),
            Err(e) => println!("⚠️  WebSocket error: {} — reconnecting in 3s", e),
        }
        sleep(Duration::from_secs(3)).await;
    }
}

async fn listen_once(cfg: &Config, sink: NewMintsSink, total_seen: &mut u64) -> Result<(), String> {
    let ws_url = format!(
        "wss://mainnet.helius-rpc.com/?api-key={}",
        cfg.helius_api_key
    );

    let (ws_stream, _) = connect_async(&ws_url)
        .await
        .map_err(|e| format!("Connect failed: {}", e))?;

    println!("✅ Helius WebSocket connected — watching pump.fun live");

    let (mut write, mut read) = ws_stream.split();

    // Subscribe to all transactions mentioning pump.fun
    let sub = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "logsSubscribe",
        "params": [
            { "mentions": [PUMP_FUN_PROGRAM] },
            { "commitment": "processed" }
        ]
    });

    write
        .send(Message::Text(sub.to_string()))
        .await
        .map_err(|e| format!("Subscribe failed: {}", e))?;

    while let Some(msg) = read.next().await {
        let msg = msg.map_err(|e| format!("Read error: {}", e))?;

        match msg {
            Message::Text(text) => {
                let Ok(val): Result<Value, _> = serde_json::from_str(&text) else { continue; };

                // Extract accounts from transaction logs
                let mints = extract_mints_from_log(&val);
                if !mints.is_empty() {
                    let mut lock = sink.lock().unwrap();
                    for mint in mints {
                        *total_seen += 1;
                        lock.push(mint.clone());
                        if *total_seen <= 10 || *total_seen % 50 == 0 {
                            println!("⚡ WS mint #{}: {}...", total_seen, &mint[..12.min(mint.len())]);
                        }
                    }
                }
            }
            Message::Ping(p) => {
                write.send(Message::Pong(p)).await.ok();
            }
            Message::Close(_) => break,
            _ => {}
        }
    }

    Ok(())
}

fn extract_mints_from_log(val: &Value) -> Vec<String> {
    let mut mints = Vec::new();

    // Check if this is a log notification
    let logs = match val["params"]["result"]["value"]["logs"].as_array() {
        Some(l) => l,
        None => return mints,
    };

    // Also check account keys from the transaction
    if let Some(accounts) = val["params"]["result"]["value"]["transaction"]["message"]["accountKeys"].as_array() {
        for acc in accounts {
            if let Some(key) = acc.as_str() {
                if is_plausible_mint(key) {
                    mints.push(key.to_string());
                }
            }
        }
    }

    // Parse logs for "Create" or "InitializeMint" patterns
    for log in logs {
        let s = log.as_str().unwrap_or("");
        if s.contains("Create") || s.contains("InitializeMint") || s.contains("MintTo") {
            // Extract any base58 tokens that look like mint addresses
            for token in s.split_whitespace() {
                if is_plausible_mint(token) {
                    mints.push(token.to_string());
                }
            }
        }
    }

    // Deduplicate
    mints.sort();
    mints.dedup();
    mints
}

fn is_plausible_mint(s: &str) -> bool {
    // Solana addresses: 32-44 chars, base58
    if s.len() < 32 || s.len() > 44 {
        return false;
    }
    // Must be base58
    if !s.chars().all(|c| matches!(c,
        '1'..='9' | 'A'..='H' | 'J'..='N' | 'P'..='Z' | 'a'..='k' | 'm'..='z'
    )) {
        return false;
    }
    // Exclude known program IDs and system addresses
    let known_programs = [
        "11111111111111111111111111111111",
        "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf8Ss623VQ5DA",
        "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P",
        "So11111111111111111111111111111111111111112",
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJe1bRS",
        "SysvarRent111111111111111111111111111111111",
        "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s",
    ];
    !known_programs.contains(&s)
}
