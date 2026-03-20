use axum::{routing::get, Router, Json};
use rusqlite::Connection;
use serde::Serialize;

#[derive(Serialize)]
struct DashboardData {
    calls_today: Vec<CallRow>,
    positions: Vec<PositionRow>,
    skip_reasons: Vec<SkipRow>,
    perf: PerfRow,
    active_count: i64,
    queue_count: i64,
    wallet_balance: f64,
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

async fn dashboard_data() -> Json<DashboardData> {
    let db_path = std::env::var("SQLITE_PATH")
        .unwrap_or_else(|_| "./data/STUPID_MAIN_DB_DO_NOT_TOUCH.sqlite".to_string());

    let conn = Connection::open(&db_path).unwrap();

    // Today's calls
    let today_ts = chrono::Utc::now().timestamp() - (6 * 3600); // last 6 hours
    let mut stmt = conn.prepare(
        "SELECT c.mint, c.fdv_usd, c.score, c.tag, c.ts, o.result, o.fdv_usd
         FROM calls c
         LEFT JOIN call_outcomes o ON o.mint = c.mint AND o.call_ts = c.ts
         WHERE c.ts > ?1 ORDER BY c.ts DESC LIMIT 20"
    ).unwrap();

    let calls_today: Vec<CallRow> = stmt.query_map([today_ts], |r| {
        Ok(CallRow {
            mint: r.get(0)?,
            fdv_usd: r.get(1)?,
            score: r.get(2)?,
            tag: r.get(3)?,
            ts: r.get(4)?,
            result: r.get(5)?,
            peak_fdv: r.get(6)?,
        })
    }).unwrap().flatten().collect();

    // Skip reasons last 5 min
    let now = chrono::Utc::now().timestamp();
    let mut stmt2 = conn.prepare(
        "SELECT reason, COUNT(*) as cnt, GROUP_CONCAT(mint, ',') as mints
         FROM (
             SELECT reason, mint FROM skip_debug
             WHERE ts > ?1
             AND reason IN ('LATE_ENTRY','RUNNER_NOT_MOVING','CONC_RISK')
             GROUP BY reason, mint
         )
         GROUP BY reason ORDER BY cnt DESC LIMIT 10"
    ).unwrap();

    let skip_reasons: Vec<SkipRow> = stmt2.query_map([now - 300], |r| {
        let mints_str: Option<String> = r.get(2)?;
        let mints = mints_str
            .unwrap_or_default()
            .split(',')
            .filter(|s| !s.is_empty())
            .take(5)
            .map(|s| s.to_string())
            .collect();
        Ok(SkipRow { reason: r.get(0)?, cnt: r.get(1)?, mints })
    }).unwrap().flatten().collect();

    // Perf today
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

    Json(DashboardData {
        calls_today,
        positions: vec![],
        skip_reasons,
        perf: PerfRow { win_rate, ..perf },
        active_count: 0,
        queue_count: 0,
        wallet_balance: 0.0,
    })
}

async fn index() -> axum::response::Html<String> {
    axum::response::Html(include_str!("../../dashboard.html").to_string())
}

#[tokio::main]
async fn main() {
    dotenvy::dotenv().ok();

    let app = Router::new()
        .route("/", get(index))
        .route("/api/data", get(dashboard_data));

    let listener = tokio::net::TcpListener::bind("127.0.0.1:3000").await.unwrap();
    println!("🌊 TARS Dashboard running at http://localhost:3000");
    axum::serve(listener, app).await.unwrap();
}
