use axum::{routing::{get, post}, Router, Json, extract::Query};
use rusqlite::Connection;
use serde::{Serialize, Deserialize};
use std::collections::HashMap;

#[derive(Serialize)]
struct DashboardData {
    calls_today: Vec<CallRow>,
    positions: Vec<PositionRow>,
    active_targets: Vec<ActiveRow>,
    skip_reasons: Vec<SkipRow>,
    perf: PerfRow,
    active_count: i64,
    queue_count: i64,
    wallet_balance: f64,
    tars_enabled: bool,
}

#[derive(Serialize)]
struct CallRow {
    mint: String,
    fdv_usd: f64,
    score: i32,
    tag: String,
    ts: i64,
    result: Option<String>,
    peak_fdv: Option<f64>,
}

#[derive(Serialize)]
struct SkipRow {
    reason: String,
    cnt: i64,
    mints: Vec<String>,
}

#[derive(Serialize)]
struct PerfRow {
    total: i64,
    wins: i64,
    losses: i64,
    win_rate: f64,
}

#[derive(Serialize)]
struct PositionRow {
    mint: String,
    entry_fdv: f64,
    current_fdv: f64,
    mult: f64,
    tag: String,
}

#[derive(Serialize)]
struct ActiveRow {
    mint: String,
    score: i32,
    fdv_usd: f64,
    tx_5m: i64,
    age_secs: i64,
}

async fn dashboard_data() -> Json<DashboardData> {
    let db_path = std::env::var("SQLITE_PATH")
        .unwrap_or_else(|_| "./data/solana_meme.sqlite".to_string());

    let conn = match Connection::open(&db_path) {
        Ok(c) => c,
        Err(_) => return Json(DashboardData {
            calls_today: vec![],
            positions: vec![],
            active_targets: vec![],
            skip_reasons: vec![],
            perf: PerfRow { total: 0, wins: 0, losses: 0, win_rate: 0.0 },
            active_count: 0,
            queue_count: 0,
            wallet_balance: 0.0,
            tars_enabled: false,
        }),
    };

    // Today's calls (last 6 hours)
    let today_ts = chrono::Utc::now().timestamp() - (6 * 3600);

    let calls_today: Vec<CallRow> = conn.prepare(
        "SELECT c.mint, c.fdv_usd, c.score, c.tag, c.ts, o.result, o.fdv_usd
         FROM calls c
         LEFT JOIN call_outcomes o ON o.mint = c.mint AND o.call_ts = c.ts
         WHERE c.ts > ?1 ORDER BY c.ts DESC LIMIT 20"
    ).ok().and_then(|mut stmt| {
        stmt.query_map([today_ts], |r| {
            Ok(CallRow {
                mint: r.get(0)?,
                fdv_usd: r.get(1)?,
                score: r.get(2)?,
                tag: r.get(3)?,
                ts: r.get(4)?,
                result: r.get(5)?,
                peak_fdv: r.get(6)?,
            })
        }).ok().map(|rows| rows.flatten().collect())
    }).unwrap_or_default();

    // Active targets — most recently scored coins (last 60s snapshots)
    let now_ts = chrono::Utc::now().timestamp();
    let active_targets: Vec<ActiveRow> = conn.prepare(
        "SELECT mint, MAX(score) as score, fdv_usd, tx_5m, first_seen
         FROM mint_snapshots
         WHERE ts > ?1
         GROUP BY mint
         ORDER BY score DESC
         LIMIT 10"
    ).ok().and_then(|mut stmt| {
        stmt.query_map([now_ts - 120], |r| {
            let first_seen: i64 = r.get(4)?;
            Ok(ActiveRow {
                mint: r.get(0)?,
                score: r.get(1)?,
                fdv_usd: r.get(2)?,
                tx_5m: r.get(3)?,
                age_secs: now_ts - first_seen,
            })
        }).ok().map(|rows| rows.flatten().collect())
    }).unwrap_or_default();

    // Live positions
    let positions: Vec<PositionRow> = conn.prepare(
        "SELECT mint, entry_fdv, COALESCE(current_fdv, entry_fdv), sol_spent, close_reason
         FROM tars_positions ORDER BY opened_ts DESC LIMIT 20"
    ).ok().and_then(|mut stmt| {
        stmt.query_map([], |r| {
            let entry: f64 = r.get(1)?;
            let current: f64 = r.get(2)?;
            let close_reason: Option<String> = r.get(4)?;
            let mult = if entry > 0.0 { current / entry } else { 1.0 };
            Ok(PositionRow {
                mint: r.get(0)?,
                entry_fdv: entry,
                current_fdv: current,
                mult,
                tag: close_reason.unwrap_or_else(|| "OPEN".to_string()),
            })
        }).ok().map(|rows| rows.flatten().collect())
    }).unwrap_or_default();

    // Skip reasons — graceful if table missing
    let skip_reasons: Vec<SkipRow> = conn.prepare(
        "SELECT reason, COUNT(*) as cnt, GROUP_CONCAT(mint, ',') as mints
         FROM (
             SELECT reason, mint FROM skip_debug
             WHERE ts > ?1
             AND reason IN ('LATE_ENTRY','RUNNER_NOT_MOVING','CONC_RISK','FDV_TOO_LOW','FDV_TOO_HIGH')
             GROUP BY reason, mint
         )
         GROUP BY reason ORDER BY cnt DESC LIMIT 10"
    ).ok().and_then(|mut stmt| {
        let now = chrono::Utc::now().timestamp();
        stmt.query_map([now - 300], |r| {
            let mints_str: Option<String> = r.get(2)?;
            let mints = mints_str
                .unwrap_or_default()
                .split(',')
                .filter(|s| !s.is_empty())
                .take(5)
                .map(|s| s.to_string())
                .collect();
            Ok(SkipRow { reason: r.get(0)?, cnt: r.get(1)?, mints })
        }).ok().map(|rows| rows.flatten().collect())
    }).unwrap_or_default();

    // Perf
    let perf = conn.query_row(
        "SELECT COUNT(*),
                SUM(CASE WHEN result='win' THEN 1 ELSE 0 END),
                SUM(CASE WHEN result='loss' THEN 1 ELSE 0 END)
         FROM call_outcomes WHERE call_ts > ?1",
        [today_ts],
        |r| Ok(PerfRow {
            total: r.get(0)?,
            wins: r.get::<_, Option<i64>>(1)?.unwrap_or(0),
            losses: r.get::<_, Option<i64>>(2)?.unwrap_or(0),
            win_rate: 0.0,
        })
    ).unwrap_or(PerfRow { total: 0, wins: 0, losses: 0, win_rate: 0.0 });

    let win_rate = if perf.total > 0 {
        (perf.wins as f64 / perf.total as f64) * 100.0
    } else { 0.0 };

    let tars_enabled = std::env::var("TARS_ENABLED")
        .unwrap_or_default()
        .to_lowercase() == "true";

    let active_count = active_targets.len() as i64;

    Json(DashboardData {
        calls_today,
        positions,
        active_targets,
        skip_reasons,
        perf: PerfRow { win_rate, ..perf },
        active_count,
        queue_count: 0,
        wallet_balance: 0.0,
        tars_enabled,
    })
}

#[derive(Serialize)]
struct BuyResult {
    ok: bool,
    message: String,
}

async fn manual_buy(Query(params): Query<HashMap<String, String>>) -> Json<BuyResult> {
    let mint = match params.get("mint") {
        Some(m) if !m.is_empty() => m.clone(),
        _ => return Json(BuyResult { ok: false, message: "missing mint".into() }),
    };

    let pub_key = std::env::var("PUMPPORTAL_PUBLIC_KEY").unwrap_or_default();
    let priv_key = std::env::var("PUMPPORTAL_PRIVATE_KEY").unwrap_or_default();
    let rpc = std::env::var("HELIUS_RPC_URL")
        .unwrap_or_else(|_| "https://api.mainnet-beta.solana.com".into());
    let sol: f64 = params.get("sol")
        .and_then(|s| s.parse().ok())
        .unwrap_or_else(|| std::env::var("TARS_BUY_SOL").ok()
            .and_then(|s| s.parse().ok()).unwrap_or(0.05));

    if priv_key.is_empty() {
        return Json(BuyResult { ok: false, message: "PUMPPORTAL_PRIVATE_KEY not set".into() });
    }

    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build().unwrap();

    let body = serde_json::json!({
        "publicKey": pub_key,
        "action": "buy",
        "mint": mint,
        "amount": sol,
        "denominatedInSol": "true",
        "slippage": 15,
        "priorityFee": 0.00005,
        "pool": "auto"
    });

    let resp = match client.post("https://pumpportal.fun/api/trade-local").json(&body).send().await {
        Ok(r) => r,
        Err(e) => return Json(BuyResult { ok: false, message: format!("request failed: {e}") }),
    };

    if !resp.status().is_success() {
        return Json(BuyResult { ok: false, message: format!("PumpPortal error: {}", resp.status()) });
    }

    // Sign and submit
    use base64::Engine;
    let tx_bytes = match resp.bytes().await {
        Ok(b) => b,
        Err(e) => return Json(BuyResult { ok: false, message: format!("bytes: {e}") }),
    };

    let keypair = match load_keypair(&priv_key) {
        Ok(k) => k,
        Err(e) => return Json(BuyResult { ok: false, message: format!("keypair: {e}") }),
    };

    let mut tx: solana_sdk::transaction::VersionedTransaction = match bincode::deserialize(&tx_bytes) {
        Ok(t) => t,
        Err(e) => return Json(BuyResult { ok: false, message: format!("deserialize: {e}") }),
    };

    let sig = solana_sdk::signature::Signer::sign_message(&keypair, &tx.message.serialize());
    tx.signatures[0] = sig;
    let tx_b64 = base64::engine::general_purpose::STANDARD.encode(bincode::serialize(&tx).unwrap());

    let rpc_resp: serde_json::Value = match client.post(&rpc).json(&serde_json::json!({
        "jsonrpc": "2.0", "id": 1, "method": "sendTransaction",
        "params": [tx_b64, {"encoding": "base64", "skipPreflight": true}]
    })).send().await {
        Ok(r) => r.json().await.unwrap_or_default(),
        Err(e) => return Json(BuyResult { ok: false, message: format!("rpc: {e}") }),
    };

    if let Some(err) = rpc_resp.get("error") {
        return Json(BuyResult { ok: false, message: format!("rpc error: {err}") });
    }

    let sig_str = rpc_resp["result"].as_str().unwrap_or("unknown").to_string();
    eprintln!("🤖 MANUAL BUY mint={} sol={} sig={}", &mint[..8], sol, &sig_str[..8.min(sig_str.len())]);

    Json(BuyResult {
        ok: true,
        message: format!("Bought {:.3} SOL of {}… sig: {}…", sol, &mint[..8], &sig_str[..8.min(sig_str.len())]),
    })
}

fn load_keypair(private_key: &str) -> Result<solana_sdk::signature::Keypair, String> {
    let key = private_key.trim();
    if let Ok(bytes) = bs58::decode(key).into_vec() {
        if bytes.len() >= 32 {
            return solana_sdk::signature::keypair_from_seed(&bytes[..32])
                .map_err(|e| e.to_string());
        }
    }
    if let Ok(bytes) = serde_json::from_str::<Vec<u8>>(key) {
        if bytes.len() >= 32 {
            return solana_sdk::signature::keypair_from_seed(&bytes[..32])
                .map_err(|e| e.to_string());
        }
    }
    Err("invalid key format".into())
}

async fn index() -> axum::response::Html<String> {
    axum::response::Html(include_str!("../../dashboard.html").to_string())
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let app = Router::new()
        .route("/", get(index))
        .route("/api/data", get(dashboard_data))
        .route("/api/buy", get(manual_buy));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
    println!("🌊 TARS Dashboard running at http://localhost:3000");
    axum::serve(listener, app).await.unwrap();
}
