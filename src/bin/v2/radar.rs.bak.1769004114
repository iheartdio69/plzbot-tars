use std::collections::HashSet;
use std::env;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde_json::json;
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

mod market;
mod state;

use market::DexSnap;
use state::{Cand, State};

#[derive(Debug, Clone)]
pub struct V2Cfg {
    pub pumpportal_enabled: bool,
    pub pumpportal_wss: String,
    pub pumpportal_api_key: String,
    pub pumpportal_channel: String,

    pub tick_secs: u64,

    pub watch_cap: usize,
    pub poll_batch: usize,
    pub top_n_print: usize,

    // Bands
    pub watch_min_fdv_usd: f64,
    pub watch_max_fdv_usd: f64,

    pub call_min_fdv_usd: f64,
    pub call_max_fdv_usd: f64,

    // Simple gates
    pub min_liq_usd: f64,
    pub min_tx_5m: u64,

    // Score threshold for emitting a CALL
    pub call_score_min: i32,

    // Dexscreener pacing
    pub per_mint_delay_ms: u64,
}

fn env_str(name: &str, default: &str) -> String {
    env::var(name).unwrap_or_else(|_| default.to_string())
}
fn env_bool(name: &str, default: bool) -> bool {
    env::var(name)
        .ok()
        .map(|v| matches!(v.to_lowercase().as_str(), "1" | "true" | "yes" | "y" | "on"))
        .unwrap_or(default)
}
fn env_u64(name: &str, default: u64) -> u64 {
    env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
fn env_usize(name: &str, default: usize) -> usize {
    env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
fn env_i32(name: &str, default: i32) -> i32 {
    env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}
fn env_f64(name: &str, default: f64) -> f64 {
    env::var(name)
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(default)
}

fn load_cfg() -> V2Cfg {
    dotenvy::dotenv().ok();

    V2Cfg {
        pumpportal_enabled: env_bool("PUMPPORTAL_ENABLED", true),
        pumpportal_wss: env_str("PUMPPORTAL_WSS", "wss://pumpportal.fun/api/data"),
        pumpportal_api_key: env_str("PUMPPORTAL_API_KEY", ""),
        pumpportal_channel: env_str("PUMPPORTAL_CHANNEL", "subscribeNewToken"),

        tick_secs: env_u64("V2_TICK_SECS", 5),

        watch_cap: env_usize("V2_WATCH_CAP", 2000),
        poll_batch: env_usize("V2_POLL_BATCH", 25),
        top_n_print: env_usize("V2_TOP_N_PRINT", 10),

        watch_min_fdv_usd: env_f64("WATCH_MIN_FDV_USD", 15_000.0),
        watch_max_fdv_usd: env_f64("WATCH_MAX_FDV_USD", 120_000.0),

        call_min_fdv_usd: env_f64("CALL_MIN_FDV_USD", 20_000.0),
        call_max_fdv_usd: env_f64("CALL_MAX_FDV_USD", 120_000.0),

        min_liq_usd: env_f64("MIN_LIQ_USD", 2_500.0),
        min_tx_5m: env_u64("MIN_TX_5M", 40),

        call_score_min: env_i32("V2_CALL_SCORE_MIN", 60),

        per_mint_delay_ms: env_u64("V2_PER_MINT_DELAY_MS", 60),
    }
}

fn short_mint(m: &str) -> String {
    if m.len() <= 10 {
        return m.to_string();
    }
    let a = &m[0..4];
    let b = &m[m.len() - 4..];
    format!("{a}…{b}")
}

fn score_of(cfg: &V2Cfg, row: &Cand) -> i32 {
    let fdv = row.fdv.unwrap_or(0.0);
    let liq = row.liq_usd.unwrap_or(0.0);
    let tx5 = row.tx_5m.unwrap_or(0) as i32;

    let mut s: i32 = 0;

    // FDV band = base signal
    if fdv >= cfg.watch_min_fdv_usd && fdv <= cfg.watch_max_fdv_usd {
        s += 25;
    }

    // Liquidity
    if liq >= cfg.min_liq_usd {
        s += 20;
    } else if liq >= (cfg.min_liq_usd * 0.5) {
        s += 10;
    }

    // Activity (tx5m)
    if tx5 >= cfg.min_tx_5m as i32 {
        s += 25;
    } else if tx5 >= (cfg.min_tx_5m as i32 / 2).max(1) {
        s += 10;
    }

    // Extra pop for higher tx
    if tx5 >= 100 {
        s += 10;
    }
    if tx5 >= 200 {
        s += 10;
    }

    s
}

fn print_queue_len(st: &State) {
    let q = st.watch_len();
    println!("QUEUE len={}", q);
}

fn print_active(cfg: &V2Cfg, st: &State) -> Vec<Cand> {
    let mut rows = st.top_candidates(200);

    rows.retain(|r| {
        let fdv = r.fdv.unwrap_or(0.0);
        let liq = r.liq_usd.unwrap_or(0.0);
        let tx5 = r.tx_5m.unwrap_or(0);

        fdv >= cfg.watch_min_fdv_usd
            && fdv <= cfg.watch_max_fdv_usd
            && liq >= cfg.min_liq_usd
            && tx5 >= cfg.min_tx_5m
    });

    rows.sort_by_key(|r| -score_of(cfg, r));
    rows.truncate(cfg.top_n_print);

    if rows.is_empty() {
        println!("ACTIVE(0): (none)");
        return rows;
    }

    let line = rows
        .iter()
        .map(|r| {
            let sc = score_of(cfg, r);
            let sym = r.symbol.clone().unwrap_or_else(|| "?".to_string());
            let fdv = r.fdv.unwrap_or(0.0);
            let tx5 = r.tx_5m.unwrap_or(0);
            format!("{sc}:{sym}@{fdv:.0}/{tx5}")
        })
        .collect::<Vec<_>>()
        .join("  ");

    println!("ACTIVE({}): {}", rows.len(), line);
    rows
}

fn print_call(mint: &str, s: &DexSnap, sc: i32) {
    let sym = s.symbol.clone().unwrap_or_else(|| "?".to_string());
    println!(
        "🚨 CALL score={} {} {} fdv=${:.0} liq=${:.0} tx5={}",
        sc,
        sym,
        short_mint(mint),
        s.fdv.unwrap_or(0.0),
        s.liq_usd.unwrap_or(0.0),
        s.tx_5m.unwrap_or(0)
    );
}

async fn pumpportal_task(cfg: V2Cfg, tx: mpsc::Sender<String>) {
    // rustls provider (ok if already installed)
    let _ = rustls::crypto::ring::default_provider().install_default();

    if !cfg.pumpportal_enabled {
        eprintln!("🟣 pumpportal disabled");
        return;
    }

    let url = cfg.pumpportal_wss.clone();
    eprintln!("🟣 pumpportal connecting: {}", url);

    loop {
        match tokio_tungstenite::connect_async(&url).await {
            Ok((ws, _)) => {
                eprintln!("✅ pumpportal connected");
                let (mut write, mut read) = ws.split();

                let mut sub = json!({ "method": cfg.pumpportal_channel });
                if !cfg.pumpportal_api_key.trim().is_empty() {
                    sub["apiKey"] = json!(cfg.pumpportal_api_key);
                }

                if write
                    .send(Message::Text(sub.to_string().into()))
                    .await
                    .is_err()
                {
                    tokio::time::sleep(Duration::from_secs(2)).await;
                    continue;
                }

                while let Some(item) = read.next().await {
                    let msg = match item {
                        Ok(m) => m,
                        Err(_) => break,
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

                    let mint = v
                        .get("mint")
                        .and_then(|x| x.as_str())
                        .or_else(|| v.get("tokenAddress").and_then(|x| x.as_str()))
                        .or_else(|| v.get("address").and_then(|x| x.as_str()))
                        .map(|s| s.trim().to_string())
                        .filter(|s| !s.is_empty());

                    if let Some(m) = mint {
                        if tx.send(m).await.is_err() {
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

        tokio::time::sleep(Duration::from_secs(2)).await;
    }
}

pub async fn run() {
    let cfg = load_cfg();

    println!("✅ V2 Runner Radar booted");
    println!(
        "Band: watch ${:.0}–${:.0} | call ${:.0}–${:.0} | min_liq=${:.0} | min_tx5={} | call_score_min={}",
        cfg.watch_min_fdv_usd,
        cfg.watch_max_fdv_usd,
        cfg.call_min_fdv_usd,
        cfg.call_max_fdv_usd,
        cfg.min_liq_usd,
        cfg.min_tx_5m,
        cfg.call_score_min
    );

    let (pump_tx, mut pump_rx) = mpsc::channel::<String>(50_000);
    tokio::spawn(pumpportal_task(cfg.clone(), pump_tx));

    let client = reqwest::Client::new();
    let mut st = State::new();

    // prevent re-calling same mint in this session
    let mut called: HashSet<String> = HashSet::new();

    loop {
        // 1) drain pumpportal
        let mut added = 0usize;
        while let Ok(mint) = pump_rx.try_recv() {
            if st.ingest_mint(mint, cfg.watch_cap) {
                added += 1;
            }
        }
        if added > 0 {
            println!("🟣 pumpportal +{} | watch_pool={}", added, st.watch_len());
        }

        // 2) poll dex for a snapshot of the watch pool
        let batch = st.watch_snapshot(cfg.poll_batch);
        for mint in batch.into_iter() {
            if let Some(s) = market::fetch_best_snap(&client, &mint).await {
                st.update_snap(&mint, &s);
            }
            if cfg.per_mint_delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(cfg.per_mint_delay_ms)).await;
            }
        }

        // 3) print queue len + active line
        print_queue_len(&st);
        let actives = print_active(&cfg, &st);

        // 4) CALL logic (simple, deterministic)
        // pick best active candidate by score, then require call band too
        if let Some(best) = actives.first() {
            let mint = best.mint.clone();

            let fdv = best.fdv.unwrap_or(0.0);
            let liq = best.liq_usd.unwrap_or(0.0);
            let tx5 = best.tx_5m.unwrap_or(0);

            let in_call_band = fdv >= cfg.call_min_fdv_usd && fdv <= cfg.call_max_fdv_usd;
            let gates_ok = in_call_band && liq >= cfg.min_liq_usd && tx5 >= cfg.min_tx_5m;

            let sc = score_of(&cfg, best);

            if gates_ok && sc >= cfg.call_score_min && !called.contains(&mint) {
                // best-effort: fetch snap again for call print
                let snap = market::fetch_best_snap(&client, &mint).await;
                if let Some(s) = snap {
                    print_call(&mint, &s, sc);
                } else {
                    // fallback print from candidate row
                    println!(
                        "🚨 CALL score={} {} {} fdv=${:.0} liq=${:.0} tx5={}",
                        sc,
                        best.symbol.clone().unwrap_or_else(|| "?".to_string()),
                        short_mint(&mint),
                        fdv,
                        liq,
                        tx5
                    );
                }
                called.insert(mint);
            }
        }

        tokio::time::sleep(Duration::from_secs(cfg.tick_secs)).await;
    }
}
