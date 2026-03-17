use clap::Parser;
use rusqlite::{params, Connection, Result as SqlResult};
use std::collections::HashMap;

#[derive(Debug)]
struct CallData {
    mint: String,
    ts: i64,
    fdv_usd: f64,
    score: i32,
    tx_5m: u64,
    signers_5m: u64,
    events: usize,
    tag: Option<String>,
    peak_fdv: Option<f64>,
    outcome: String, // "winner" | "rug" | "unknown"
}

#[derive(Debug, Default, Clone)]
struct CallAggFp {
    top1: Option<f64>,
    top5: Option<f64>,
}

#[derive(Debug)]
struct WalletData {
    wallet: String,
    score: i64,
    edge_count: i64,
}

#[derive(Debug)]
struct Recommendation {
    description: String,
    suggested_adjustment: String,
}

#[derive(Parser, Debug)]
#[command(version, about = "Brain for Solana Meme Bot")]
struct Args {
    /// Path to sqlite db (overridden by SQLITE_PATH env var)
    #[arg(long, default_value = "./data/solana_meme.sqlite")]
    db: String,

    /// Peak window in seconds after call time (default 3600 = 60m)
    #[arg(long, default_value_t = 3600)]
    peak_window_sec: i64,
}

fn main() -> SqlResult<()> {
    dotenvy::dotenv().ok();
    let args = Args::parse();

    let cwd = std::env::current_dir().unwrap();
    let env_sqlite_path = std::env::var("SQLITE_PATH").ok();

    // Decide db_path without moving args.db
    let db_path: String = env_sqlite_path.clone().unwrap_or_else(|| args.db.clone());

    println!("Brain starting...");
    println!("CWD: {}", cwd.display());
    println!("ENV SQLITE_PATH: {:?}", env_sqlite_path);
    println!("Args --db: {}", args.db);
    println!("Using DB: {}", db_path);
    println!("Reading DB: {}", db_path);

    let conn = Connection::open(&db_path)?;

    // Load data
    let calls = load_recent_calls(&conn, args.peak_window_sec)?;
    let wallets = load_monitored_wallets(&conn)?;
    let wallets_in_calls = load_wallets_in_calls(&conn)?;
    let missing = measure_missing_signers(&conn)?;
    let (total_calls, tx50_signers0, tx50_events0) = pipeline_health(&conn)?;
    let call_agg_fp = load_call_agg_fingerprints(&conn)?; // top1/top5 per call if present

    // Analyze & learn
    let recs = analyze(
        &calls,
        &wallets,
        &wallets_in_calls,
        &missing,
        &call_agg_fp,
        total_calls,
        tx50_signers0,
        tx50_events0,
    );

    // Print report
    print_brain_report(
        &calls,
        &wallets,
        &missing,
        total_calls,
        tx50_signers0,
        tx50_events0,
        &call_agg_fp,
        &recs,
    );

    Ok(())
}

// -------------------------
// DB loads
// -------------------------

fn load_recent_calls(conn: &Connection, peak_window_sec: i64) -> SqlResult<Vec<CallData>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT
            c.mint,
            c.ts,
            c.fdv_usd,
            c.score,
            c.tx_5m,
            c.signers_5m,
            c.events,
            COALESCE(NULLIF(c.tag,''), NULL) AS tag,
            -- peak fdv in window
            (
              SELECT MAX(ms.fdv_usd)
              FROM mint_snapshots ms
              WHERE ms.mint = c.mint
                AND ms.ts BETWEEN c.ts AND (c.ts + ?1)
            ) AS peak_fdv,
            o.result AS outcome_result
        FROM calls c
        LEFT JOIN call_outcomes o
          ON o.mint = c.mint AND o.call_ts = c.ts
        ORDER BY c.ts DESC
        LIMIT 200
        "#,
    )?;

    let mut rows = stmt.query(params![peak_window_sec])?;
    let mut calls = Vec::new();

    while let Some(row) = rows.next()? {
        let outcome_result: Option<String> = row.get(9)?;
        let outcome = match outcome_result.as_deref() {
            Some("win") => "winner".to_string(),
            Some("loss") => "rug".to_string(),
            _ => "unknown".to_string(),
        };

        calls.push(CallData {
            mint: row.get(0)?,
            ts: row.get(1)?,
            fdv_usd: row.get(2)?,
            score: row.get(3)?,
            tx_5m: row.get::<_, i64>(4)? as u64,
            signers_5m: row.get::<_, i64>(5)? as u64,
            events: row.get::<_, i64>(6)? as usize,
            tag: row.get(7)?,
            peak_fdv: row.get(8)?,
            outcome,
        });
    }

    Ok(calls)
}

fn load_monitored_wallets(conn: &Connection) -> SqlResult<Vec<WalletData>> {
    let sql = r#"
        SELECT w.wallet, w.score,
               (SELECT COUNT(*) FROM wallet_edges we WHERE we.src_wallet = w.wallet) AS edge_count
        FROM wallets w
        ORDER BY w.score DESC
        LIMIT 200
    "#;

    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query([])?;

    let mut wallets = Vec::new();
    while let Some(row) = rows.next()? {
        wallets.push(WalletData {
            wallet: row.get(0)?,
            score: row.get(1)?,
            edge_count: row.get(2)?,
        });
    }
    Ok(wallets)
}

fn load_wallets_in_calls(conn: &Connection) -> SqlResult<HashMap<String, Vec<String>>> {
    let sql = r#"
        SELECT c.mint, we.src_wallet
        FROM calls c
        JOIN wallet_edges we ON we.mint = c.mint
        WHERE we.ts BETWEEN c.ts - 300 AND c.ts
          AND we.src_wallet IS NOT NULL AND we.src_wallet != ''
          AND we.action IN ('pair_tx', 'token_transfer')
        GROUP BY c.mint, we.src_wallet
    "#;

    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query([])?;

    let mut map: HashMap<String, Vec<String>> = HashMap::new();
    while let Some(row) = rows.next()? {
        let mint: String = row.get(0)?;
        let wallet: String = row.get(1)?;
        map.entry(mint).or_default().push(wallet);
    }
    Ok(map)
}

fn measure_missing_signers(conn: &Connection) -> SqlResult<HashMap<String, i64>> {
    let sql = r#"
        SELECT
          COALESCE(SUM(CASE WHEN signers_5m = 0 AND tx_5m > 50 THEN 1 ELSE 0 END), 0) AS low_signers,
          COALESCE(SUM(CASE WHEN events = 0 AND tx_5m > 50 THEN 1 ELSE 0 END), 0) AS low_events
        FROM calls
    "#;

    let mut stmt = conn.prepare(sql)?;
    let (low_signers, low_events) =
        stmt.query_row([], |row| Ok((row.get::<_, i64>(0)?, row.get::<_, i64>(1)?)))?;

    let mut counts = HashMap::new();
    counts.insert("low_signers".to_string(), low_signers);
    counts.insert("low_events".to_string(), low_events);
    Ok(counts)
}

fn pipeline_health(conn: &Connection) -> SqlResult<(i64, i64, i64)> {
    let mut stmt = conn.prepare(
        r#"
        SELECT
          COUNT(*) AS total,
          SUM(CASE WHEN tx_5m > 50 AND signers_5m = 0 THEN 1 ELSE 0 END) AS tx50_signers0,
          SUM(CASE WHEN tx_5m > 50 AND events = 0 THEN 1 ELSE 0 END) AS tx50_events0
        FROM calls
        "#,
    )?;

    let row = stmt.query_row([], |r| Ok((r.get(0)?, r.get(1)?, r.get(2)?)))?;
    Ok(row)
}

fn load_call_agg_fingerprints(conn: &Connection) -> SqlResult<HashMap<(String, i64), CallAggFp>> {
    let mut stmt = conn.prepare(
        r#"
        SELECT mint, call_ts, reason, metric
        FROM call_wallet_fingerprints
        WHERE wallet = '__agg__'
AND reason IN ('top1_share', 'top5_share')        "#,
    )?;

    let mut rows = stmt.query([])?;
    let mut map: HashMap<(String, i64), CallAggFp> = HashMap::new();

    while let Some(r) = rows.next()? {
        let mint: String = r.get(0)?;
        let call_ts: i64 = r.get(1)?;
        let reason: String = r.get(2)?;
        let metric: f64 = r.get(3)?;

        let entry = map.entry((mint, call_ts)).or_default();
        if reason == "top1_share" {
            entry.top1 = Some(metric);
        }
        if reason == "top5_share" {
            entry.top5 = Some(metric);
        }
    }

    Ok(map)
}

// -------------------------
// Analysis
// -------------------------

fn analyze(
    calls: &[CallData],
    wallets: &[WalletData],
    wallets_in_calls: &HashMap<String, Vec<String>>,
    missing: &HashMap<String, i64>,
    call_agg_fp: &HashMap<(String, i64), CallAggFp>,
    total_calls: i64,
    tx50_signers0: i64,
    tx50_events0: i64,
) -> Vec<Recommendation> {
    let mut recs = Vec::new();

    let winners: Vec<&CallData> = calls.iter().filter(|c| c.outcome == "winner").collect();
    let rugs: Vec<&CallData> = calls.iter().filter(|c| c.outcome == "rug").collect();

    recs.push(Recommendation {
        description: "Sample size".to_string(),
        suggested_adjustment: format!(
            "calls={} winners={} rugs={} unknown={}",
            calls.len(),
            winners.len(),
            rugs.len(),
            calls.iter().filter(|c| c.outcome == "unknown").count()
        ),
    });

    // FDV band from winners (only if enough data)
    if winners.len() >= 10 {
        let mut fdvs: Vec<f64> = winners.iter().map(|c| c.fdv_usd).collect();
        fdvs.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let p20 = percentile(&fdvs, 0.2);
        let p80 = percentile(&fdvs, 0.8);

        recs.push(Recommendation {
            description: "FDV percentiles from WINNER calls".to_string(),
            suggested_adjustment: format!(
                "Set call band: min_call_fdv_usd≈{:.0}, max_call_fdv_usd≈{:.0}",
                p20, p80
            ),
        });
    } else {
        recs.push(Recommendation {
            description: "FDV band".to_string(),
            suggested_adjustment:
                "Not enough winners to retune FDV band yet (need ~10+ winner samples)".to_string(),
        });
    }

    // Signers guidance from winners
    if !winners.is_empty() {
        let avg_signers =
            winners.iter().map(|c| c.signers_5m as f64).sum::<f64>() / winners.len() as f64;
        recs.push(Recommendation {
            description: "Avg signers_5m in WINNER calls".to_string(),
            suggested_adjustment: format!("avg_signers_5m≈{:.1}", avg_signers),
        });
    }

    // Pipeline health
    recs.push(Recommendation {
        description: "Signal health (calls table)".to_string(),
        suggested_adjustment: format!(
            "total_calls={} | tx>50 & signers=0 => {} | tx>50 & events=0 => {}",
            total_calls, tx50_signers0, tx50_events0
        ),
    });

    // Concentration summary on rugs (only where we have agg fp)
    if !rugs.is_empty() {
        let mut n = 0usize;
        let mut top1_sum = 0.0;
        let mut top5_sum = 0.0;

        for c in rugs {
            let key = (c.mint.clone(), c.ts);

            if let Some(fp) = call_agg_fp.get(&key) {
                if let (Some(t1), Some(t5)) = (fp.top1, fp.top5) {
                    top1_sum += t1;
                    top5_sum += t5;
                    n += 1;
                }
            }
        }

        if n > 0 {
            recs.push(Recommendation {
                description: "RUG concentration (from __agg__ fingerprints)".to_string(),
                suggested_adjustment: format!(
                    "avg_top1≈{:.3}, avg_top5≈{:.3}. Consider gates like top1>=0.22 OR top5>=0.55 unless override_ok.",
                    top1_sum / n as f64,
                    top5_sum / n as f64
                ),
            });
        } else {
            recs.push(Recommendation {
                description: "RUG concentration".to_string(),
                suggested_adjustment:
                    "No __agg__ fingerprints found for rugs yet (ensure you write call_wallet_fingerprints on each call)".to_string(),
            });
        }
    }

    // High-risk wallets exist?
    let high_risk = wallets.iter().filter(|w| w.score <= -50).count();
    if high_risk > 0 {
        recs.push(Recommendation {
            description: "High-risk wallets (score <= -50) exist".to_string(),
            suggested_adjustment: format!(
                "count={}. Consider heavier penalty or hard-skip if these appear early in a mint.",
                high_risk
            ),
        });
    }

    // (Optional) “wallets seen per call” sanity
    let mut with_wallets = 0usize;
    for c in calls.iter().take(200) {
        if wallets_in_calls
            .get(&c.mint)
            .map(|v| !v.is_empty())
            .unwrap_or(false)
        {
            with_wallets += 1;
        }
    }
    recs.push(Recommendation {
        description: "Wallets-in-calls coverage".to_string(),
        suggested_adjustment: format!(
            "{} / {} recent calls had at least 1 wallet edge in the -5m window",
            with_wallets,
            calls.len().min(200)
        ),
    });

    // Missing counts from measure_missing_signers (same idea, different aggregation)
    let low_signers = *missing.get("low_signers").unwrap_or(&0);
    let low_events = *missing.get("low_events").unwrap_or(&0);
    recs.push(Recommendation {
        description: "Missing signal counts".to_string(),
        suggested_adjustment: format!("low_signers={} | low_events={}", low_signers, low_events),
    });

    recs
}

fn percentile(data: &[f64], p: f64) -> f64 {
    if data.is_empty() {
        return 0.0;
    }
    let p = p.clamp(0.0, 1.0);
    let idx = ((data.len() as f64 - 1.0) * p).round() as usize;
    data[idx.min(data.len() - 1)]
}

// -------------------------
// Report
// -------------------------

fn print_brain_report(
    calls: &[CallData],
    wallets: &[WalletData],
    missing: &HashMap<String, i64>,
    total_calls: i64,
    tx50_signers0: i64,
    tx50_events0: i64,
    call_agg_fp: &HashMap<(String, i64), CallAggFp>,
    recs: &[Recommendation],
) {
    println!("\n=== BRAIN REPORT ===\n");
    println!(
        "Analyzed {} calls, {} wallets\n",
        calls.len(),
        wallets.len()
    );

    println!("Signal health (calls table):");
    println!(
        "total_calls={} | tx>50 & signers=0={} | tx>50 & events=0={}",
        total_calls, tx50_signers0, tx50_events0
    );
    println!(
        "low_signers(tx>50 & signers=0)={} | low_events(tx>50 & events=0)={}",
        missing.get("low_signers").unwrap_or(&0),
        missing.get("low_events").unwrap_or(&0)
    );

    println!("\nRecent Calls (last 8):");
    for c in calls.iter().take(8) {
        let peak = c
            .peak_fdv
            .map(|v| format!("{:.0}", v))
            .unwrap_or_else(|| "-".to_string());

        let fp = call_agg_fp.get(&(c.mint.clone(), c.ts));
        let (t1, t5) = if let Some(fp) = fp {
            (
                fp.top1
                    .map(|x| format!("{:.3}", x))
                    .unwrap_or_else(|| "-".to_string()),
                fp.top5
                    .map(|x| format!("{:.3}", x))
                    .unwrap_or_else(|| "-".to_string()),
            )
        } else {
            ("-".to_string(), "-".to_string())
        };

        println!(
            "Mint: {} | callFDV:${:.0} | peak:${} | outcome:{} | signers:{} | tx5:{} | ev:{} | top1:{} top5:{} | tag:{:?}",
            c.mint, c.fdv_usd, peak, c.outcome, c.signers_5m, c.tx_5m, c.events, t1, t5, c.tag
        );
    }

    println!("\nTop Wallets (first 10 by score):");
    for w in wallets.iter().take(10) {
        println!(
            "Wallet: {} | score:{} | edges:{}",
            w.wallet, w.score, w.edge_count
        );
    }

    println!("\nLearned Insights & Suggested Adjustments:");
    for r in recs {
        println!("→ {}: {}", r.description, r.suggested_adjustment);
    }
}
