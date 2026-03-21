// helius/websocket.rs
// Subscribes to Solana transaction logs for pump.fun program.
// Catches new token mints the SECOND they happen — not minutes later via DexScreener.

use crate::config::Config;
use crate::types::CoinState;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use tokio::time::{sleep, Duration};

// pump.fun program ID
const PUMP_FUN_PROGRAM: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";

pub type SharedCoins = Arc<Mutex<HashMap<String, CoinState>>>;

pub async fn run_websocket_listener(cfg: Config, coins: SharedCoins) {
    loop {
        println!("🔌 Helius WebSocket connecting...");
        match listen_once(&cfg, coins.clone()).await {
            Ok(_) => println!("WebSocket disconnected — reconnecting..."),
            Err(e) => println!("WebSocket error: {} — reconnecting in 5s", e),
        }
        sleep(Duration::from_secs(5)).await;
    }
}

async fn listen_once(cfg: &Config, coins: SharedCoins) -> Result<(), String> {
    use tokio_tungstenite::connect_async;
    use futures_util::{SinkExt, StreamExt};
    use serde_json::{json, Value};

    let ws_url = format!(
        "wss://mainnet.helius-rpc.com/?api-key={}",
        cfg.helius_api_key
    );

    let (ws_stream, _) = connect_async(&ws_url)
        .await
        .map_err(|e| format!("Connect failed: {}", e))?;

    println!("✅ Helius WebSocket connected — watching pump.fun");

    let (mut write, mut read) = ws_stream.split();

    // Subscribe to pump.fun logs
    let subscribe_msg = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "logsSubscribe",
        "params": [
            { "mentions": [PUMP_FUN_PROGRAM] },
            { "commitment": "processed" }
        ]
    });

    write.send(tokio_tungstenite::tungstenite::Message::Text(
        subscribe_msg.to_string()
    )).await.map_err(|e| format!("Subscribe failed: {}", e))?;

    let mut new_mints = 0u64;

    while let Some(msg) = read.next().await {
        let msg = msg.map_err(|e| format!("Read error: {}", e))?;
        let text = match msg {
            tokio_tungstenite::tungstenite::Message::Text(t) => t,
            tokio_tungstenite::tungstenite::Message::Ping(p) => {
                write.send(tokio_tungstenite::tungstenite::Message::Pong(p)).await.ok();
                continue;
            }
            _ => continue,
        };

        let Ok(val): Result<Value, _> = serde_json::from_str(&text) else { continue; };

        // Extract mint addresses from log messages
        if let Some(logs) = val["params"]["result"]["value"]["logs"].as_array() {
            for log in logs {
                let log_str = log.as_str().unwrap_or("");
                // pump.fun logs contain "InitializeMint" or mint address patterns
                if log_str.contains("InitializeMint") || log_str.contains("create") {
                    // Extract any base58 addresses from the log
                    for token in log_str.split_whitespace() {
                        if token.len() >= 32 && token.len() <= 44 && is_base58(token) {
                            let mut lock = coins.lock().unwrap();
                            if !lock.contains_key(token) {
                                lock.insert(token.to_string(), CoinState::new_with_mint(token.to_string()));
                                new_mints += 1;
                                if new_mints % 10 == 0 || new_mints <= 5 {
                                    println!("⚡ WS new mint #{}: {}", new_mints, &token[..12]);
                                }
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}

fn is_base58(s: &str) -> bool {
    s.chars().all(|c| {
        matches!(c, '1'..='9' | 'A'..='H' | 'J'..='N' | 'P'..='Z' | 'a'..='k' | 'm'..='z')
    })
}
