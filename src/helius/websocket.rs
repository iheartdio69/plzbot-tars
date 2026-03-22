// helius/websocket.rs — with exponential backoff reconnection

use crate::config::Config;
use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::time::sleep;
use tokio_tungstenite::connect_async;
use tokio_tungstenite::tungstenite::Message;

pub type NewMintsSink = Arc<Mutex<Vec<String>>>;

const PUMP_FUN_PROGRAM: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
const MAX_BACKOFF_SECS: u64 = 60;

pub async fn subscribe_pump_fun(cfg: &Config, sink: NewMintsSink) {
    let mut total_seen = 0u64;
    let mut backoff_secs = 1u64;
    let mut attempt = 0u32;

    loop {
        attempt += 1;
        println!("🔌 Helius WebSocket connecting (attempt {})...", attempt);

        match listen_once(cfg, sink.clone(), &mut total_seen).await {
            Ok(_) => {
                println!("⚠️  WebSocket disconnected cleanly — reconnecting...");
                backoff_secs = 1; // reset on clean disconnect
            }
            Err(e) => {
                println!("⚠️  WebSocket error: {} — retrying in {}s", e, backoff_secs);
                sleep(Duration::from_secs(backoff_secs)).await;
                // Exponential backoff: 1, 2, 4, 8, 16, 32, 60, 60, 60...
                backoff_secs = (backoff_secs * 2).min(MAX_BACKOFF_SECS);
                continue;
            }
        }

        // Small delay before reconnect after clean disconnect
        sleep(Duration::from_secs(2)).await;
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

    // Ping keepalive task
    let ping_interval = Duration::from_secs(30);
    let mut last_ping = tokio::time::Instant::now();

    while let Some(msg) = read.next().await {
        let msg = msg.map_err(|e| format!("Read error: {}", e))?;

        // Send keepalive ping
        if last_ping.elapsed() >= ping_interval {
            let _ = write.send(Message::Ping(vec![])).await;
            last_ping = tokio::time::Instant::now();
        }

        match msg {
            Message::Text(text) => {
                let Ok(val): Result<Value, _> = serde_json::from_str(&text) else { continue; };
                let mints = extract_mints_from_log(&val);
                if !mints.is_empty() {
                    let mut lock = sink.lock().unwrap();
                    for mint in mints {
                        *total_seen += 1;
                        lock.push(mint.clone());
                        if *total_seen <= 5 || *total_seen % 100 == 0 {
                            println!("⚡ WS mint #{}: {}...", total_seen, &mint[..12.min(mint.len())]);
                        }
                    }
                }
            }
            Message::Ping(p) => { let _ = write.send(Message::Pong(p)).await; }
            Message::Close(_) => break,
            _ => {}
        }
    }

    Ok(())
}

fn extract_mints_from_log(val: &Value) -> Vec<String> {
    let mut mints = Vec::new();

    if let Some(accounts) = val["params"]["result"]["value"]["transaction"]["message"]["accountKeys"].as_array() {
        for acc in accounts {
            if let Some(key) = acc.as_str() {
                if is_plausible_mint(key) {
                    mints.push(key.to_string());
                }
            }
        }
    }

    let logs = match val["params"]["result"]["value"]["logs"].as_array() {
        Some(l) => l,
        None => return mints,
    };

    for log in logs {
        let s = log.as_str().unwrap_or("");
        if s.contains("Create") || s.contains("InitializeMint") {
            for token in s.split_whitespace() {
                if is_plausible_mint(token) {
                    mints.push(token.to_string());
                }
            }
        }
    }

    mints.sort();
    mints.dedup();
    mints
}

fn is_plausible_mint(s: &str) -> bool {
    if s.len() < 32 || s.len() > 44 { return false; }
    if !s.chars().all(|c| matches!(c, '1'..='9' | 'A'..='H' | 'J'..='N' | 'P'..='Z' | 'a'..='k' | 'm'..='z')) {
        return false;
    }
    const KNOWN_PROGRAMS: &[&str] = &[
        "11111111111111111111111111111111",
        "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf8Ss623VQ5DA",
        "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P",
        "So11111111111111111111111111111111111111112",
        "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v",
        "ATokenGPvbdGVxr1b2hvZbsiqW5xWH25efTNsLJe1bRS",
        "SysvarRent111111111111111111111111111111111",
        "metaqbxxUerdq28cj1RbAWkYQm3ybzjb6a8bt518x1s",
    ];
    !KNOWN_PROGRAMS.contains(&s)
}
