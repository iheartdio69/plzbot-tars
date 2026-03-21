// src/pumpportal/client.rs
use crate::config::Config;
use crate::governor::Governor;
use crate::pumpportal::types::{PumpMint, PumpMintMeta, PumpTrade};
use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

pub async fn run(
    cfg: Config,
    tx: mpsc::Sender<PumpMint>,
    trade_tx: mpsc::Sender<PumpTrade>,
    gov: Arc<Governor>,
) {
    let _ = gov;

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

                // Subscribe to new tokens
                let mut sub_new = json!({ "method": cfg.pumpportal_channel });
                if !cfg.pumpportal_api_key.trim().is_empty() {
                    sub_new["apiKey"] = json!(cfg.pumpportal_api_key);
                }
                if let Err(e) = write.send(Message::Text(sub_new.to_string().into())).await {
                    eprintln!("❌ pumpportal subscribe(newToken) failed: {}", e);
                    tokio::time::sleep(std::time::Duration::from_secs(2)).await;
                    continue;
                }

                // Also subscribe to all trades — gives us real wallet addresses in real-time
                // This is the key signal for pre-graduation coins (no Helius needed)
                let sub_trades = json!({ "method": "subscribeTokenTrade" });
                if let Err(e) = write.send(Message::Text(sub_trades.to_string().into())).await {
                    eprintln!("⚠️ pumpportal subscribe(tokenTrade) failed (non-fatal): {}", e);
                    // Continue anyway — new token stream still works
                }

                eprintln!("🟣 pumpportal subscribed: newToken + tokenTrade");

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

                    if std::env::var("DEBUG_PP").ok().as_deref() == Some("1") {
                        eprintln!("DBG PP RAW: {}", &text[..text.len().min(500)]);
                    }

                    let v: serde_json::Value = match serde_json::from_str(&text) {
                        Ok(v) => v,
                        Err(_) => continue,
                    };

                    // Detect message type: trade events have "txType" or "traderPublicKey" + "mint"
                    let tx_type = v.get("txType").and_then(|x| x.as_str()).unwrap_or("");
                    let has_trader = v.get("traderPublicKey").is_some();
                    let has_mint = v.get("mint").is_some();

                    if has_mint && has_trader && (tx_type == "buy" || tx_type == "sell" || tx_type == "create") {
                        // This is a trade event
                        if let (Some(mint), Some(trader)) = (
                            v.get("mint").and_then(|x| x.as_str()),
                            v.get("traderPublicKey").and_then(|x| x.as_str()),
                        ) {
                            let sol_amount = v.get("solAmount")
                                .and_then(|x| x.as_f64())
                                .unwrap_or(0.0) / 1e9; // lamports → SOL

                            let is_buy = tx_type != "sell";
                            let market_cap_sol = v.get("marketCapSol").and_then(|x| x.as_f64());
                            let ts = v.get("timestamp").and_then(|x| x.as_u64())
                                .unwrap_or_else(crate::time::now);

                            let trade = PumpTrade {
                                mint: mint.to_string(),
                                trader: trader.to_string(),
                                sol_amount,
                                is_buy,
                                market_cap_sol,
                                ts,
                            };

                            if trade_tx.send(trade).await.is_err() {
                                eprintln!("🟣 pumpportal trade_tx dropped; stopping");
                                return;
                            }
                        }
                    } else {
                        // New token event
                        let mint = v.get("mint")
                            .and_then(|x| x.as_str())
                            .or_else(|| v.get("tokenAddress").and_then(|x| x.as_str()))
                            .or_else(|| v.get("address").and_then(|x| x.as_str()))
                            .map(|s| s.trim().to_string())
                            .filter(|s| !s.is_empty());

                        if let Some(mint) = mint {
                            let meta = PumpMintMeta {
                                name: v.get("name").and_then(|x| x.as_str()).map(|s| s.to_string()),
                                symbol: v.get("symbol").and_then(|x| x.as_str()).map(|s| s.to_string()),
                                description: v.get("description").and_then(|x| x.as_str()).map(|s| s.to_string()),
                                twitter: v.get("twitter").and_then(|x| x.as_str()).filter(|s| !s.is_empty()).map(|s| s.to_string()),
                                telegram: v.get("telegram").and_then(|x| x.as_str()).filter(|s| !s.is_empty()).map(|s| s.to_string()),
                                website: v.get("website").and_then(|x| x.as_str()).filter(|s| !s.is_empty()).map(|s| s.to_string()),
                                image_uri: v.get("uri").and_then(|x| x.as_str()).map(|s| s.to_string()),
                            };

                            if meta.has_socials() {
                                eprintln!(
                                    "🌐 social mint={} twitter={} tg={} web={}",
                                    &mint[..8.min(mint.len())],
                                    meta.twitter.as_deref().unwrap_or("-"),
                                    meta.telegram.as_deref().unwrap_or("-"),
                                    meta.website.as_deref().unwrap_or("-"),
                                );
                            }

                            let pump_mint = PumpMint {
                                mint,
                                market_cap_sol: v.get("marketCapSol").and_then(|x| x.as_f64()),
                                v_sol_in_bonding_curve: v.get("vSolInBondingCurve").and_then(|x| x.as_f64()),
                                v_tokens_in_bonding_curve: v.get("vTokensInBondingCurve").and_then(|x| x.as_f64()),
                                creator: v.get("traderPublicKey").and_then(|x| x.as_str()).map(|s| s.to_string()),
                                meta,
                            };

                            if tx.send(pump_mint).await.is_err() {
                                eprintln!("🟣 pumpportal receiver dropped; stopping");
                                return;
                            }
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
