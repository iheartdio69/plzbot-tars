// src/pumpportal/client.rs
use crate::config::Config;
use crate::governor::Governor;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

pub async fn run(cfg: Config, tx: mpsc::Sender<String>, gov: Arc<Governor>) {
    // PumpPortal websocket traffic is NOT Helius credits, but we keep `gov`
    // in the signature so the whole app wiring stays consistent.
    let _ = gov;

    // rustls 0.23 requires selecting a crypto provider explicitly
    // (safe to call multiple times; we just ignore failure)
    if let Err(e) = rustls::crypto::ring::default_provider().install_default() {
        eprintln!("⚠️ rustls crypto provider install failed (ok to ignore): {e:?}");
    }

    if !cfg.pumpportal_enabled {
        eprintln!("🟣 pumpportal disabled (pumpportal_enabled=false)");
        return;
    }

    let url = cfg.pumpportal_wss.clone();
    eprintln!("🟣 pumpportal connecting: {}", url);

    loop {
        match tokio_tungstenite::connect_async(&url).await {
            Ok((ws, _resp)) => {
                eprintln!("✅ pumpportal connected");

                let (mut write, mut read) = ws.split();

                // Subscription payload
                // Typical channel: "subscribeNewToken" (whatever your cfg uses)
                let mut sub = json!({
                    "method": cfg.pumpportal_channel,
                });

                // Optional api key support (some services ignore it)
                if !cfg.pumpportal_api_key.trim().is_empty() {
                    sub["apiKey"] = json!(cfg.pumpportal_api_key);
                }

                if let Err(e) = write.send(Message::Text(sub.to_string().into())).await {
                    eprintln!("❌ pumpportal subscribe send failed: {}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }

                // Read loop
                while let Some(item) = read.next().await {
                    let msg = match item {
                        Ok(m) => m,
                        Err(e) => {
                            eprintln!("❌ pumpportal read error: {}", e);
                            break;
                        }
                    };

                    let text: String = match msg {
                        Message::Text(t) => t.to_string(),
                        Message::Binary(b) => String::from_utf8_lossy(&b).to_string(),
                        _ => continue,
                    };

                    let v: serde_json::Value = match serde_json::from_str(&text) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    // Common shapes: { "mint": "..." } or { "tokenAddress": "..." } or { "address": "..." }
                    let mint = v
                        .get("mint")
                        .and_then(|x| x.as_str())
                        .or_else(|| v.get("tokenAddress").and_then(|x| x.as_str()))
                        .or_else(|| v.get("address").and_then(|x| x.as_str()))
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());

                    if let Some(mint) = mint {
                        // Best-effort send; if receiver is dropped, just stop the task.
                        if tx.send(mint).await.is_err() {
                            eprintln!("🟣 pumpportal receiver dropped; stopping task");
                            return;
                        }
                    }
                }

                eprintln!("🟣 pumpportal disconnected; reconnecting…");
            }
            Err(e) => {
                eprintln!("❌ pumpportal connect failed: {}", e);
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}
