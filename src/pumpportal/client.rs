use crate::config::Config;
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

pub async fn run(cfg: Config, tx: mpsc::Sender<String>) {
    // rustls 0.23 requires selecting a crypto provider explicitly
    let _ = rustls::crypto::ring::default_provider().install_default();

    if !cfg.pumpportal_enabled {
        return;
    }

    let url = cfg.pumpportal_wss.clone();
    eprintln!("🧪 pumpportal connecting: {}", url);

    loop {
        match tokio_tungstenite::connect_async(&url).await {
            Ok((ws, _resp)) => {
                eprintln!("✅ pumpportal connected");

                let (mut write, mut read) = ws.split();

                // subscription payload
                // channel default: subscribeNewToken
                let mut sub = json!({
                    "method": cfg.pumpportal_channel,
                });

                // optional api key support (some services ignore it)
                if !cfg.pumpportal_api_key.is_empty() {
                    sub["apiKey"] = json!(cfg.pumpportal_api_key);
                }

                if let Err(e) = write.send(Message::Text(sub.to_string().into())).await {
                    eprintln!("❌ pumpportal subscribe send failed: {}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }

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

                    // common shapes: { "mint": "..." } or { "tokenAddress": "..." } etc
                    let mint = v
                        .get("mint")
                        .and_then(|x| x.as_str())
                        .or_else(|| v.get("tokenAddress").and_then(|x| x.as_str()))
                        .or_else(|| v.get("address").and_then(|x| x.as_str()))
                        .map(|s| s.to_string());

                    if let Some(mint) = mint {
                        let _ = tx.send(mint).await;
                    }
                }
            }
            Err(e) => {
                eprintln!("❌ pumpportal connect failed: {}", e);
            }
        }

        tokio::time::sleep(std::time::Duration::from_secs(2)).await;
    }
}
