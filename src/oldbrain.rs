use clap::Parser;
use rusqlite::Connection;
use std::collections::HashMap;
use std::env;

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
    outcome: String, // "winner" | "meh" | "rug" | "unknown"
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
#[command(version, about = "Adaptive Brain for Solana Meme Bot")]
struct Args {
    /// Path to sqlite db (overridden by SQLITE_PATH env var)
    #[arg(long, default_value = "./bot.db")]
    db: String,

    /// Peak window in seconds after call time (default 3600 = 60m)
    #[arg(long, default_value_t = 3600)]
    peak_window_sec: i64,
}

fn main() -> rusqlite::Result<()> {
    let args = Args::parse();
    let db_path = env::var("SQLITE_PATH").unwrap_or(args.db);

    println!("Adaptive Brain starting...");
    println!("Reading DB: {}", db_path);

    let conn = Connection::open(db_path)?;

    // Load data
    let calls = load_recent_calls(&conn, args.peak_window_sec)?;
    let wallets = load_monitored_wallets(&conn)?;
    let wallets_in_calls = load_wallets_in_calls(&conn)?;
    let missing = measure_missing_signers(&conn)?;

    // Analyze & learn
    let recs = analyze(&conn, &calls, &wallets, &wallets_in_calls, &missing);

    // Print report
    print_brain_report(&calls, &wallets, &missing, &recs);

    Ok(())
}

fn load_recent_calls(conn: &Connection, peak_window_sec: i64) -> rusqlite::Result<Vec<CallData>> {
    let sql = r#"
        SELECT
            c.mint, c.ts, c.fdv_usd, c.score, c.tx_5m, c.signers_5m, c.events, c.tag,
            (
              SELECT MAX(ms.fdv_usd)
              FROM mint_snapshots ms
              WHERE ms.mint = c.mint
                AND ms.ts > c.ts
                AND ms.ts <= c.ts + ?
            ) AS peak_fdv
        FROM calls c
        ORDER BY c.ts DESC
        LIMIT 200
    "#;

    let mut stmt = conn.prepare(sql)?;
    let mut rows = stmt.query([peak_window_sec])?;

    let mut calls = Vec::new();
    while let Some(row) = rows.next()? {
        let call_fdv: f64 = row.get(2)?;
        let peak_fdv: Option<f64> = row.get(8)?;

        // outcome buckets
        let outcome = match peak_fdv {
            None => "unknown".to_string(),
            Some(p) if p >= call_fdv * 2.0 => "winner".to_string(),
            Some(p) if p >= call_fdv * 1.2 => "meh".to_string(),
            Some(_) => "rug".to_string(),
        };

        calls.push(CallData {
            mint: row.get(0)?,
            ts: row.get(1)?,
            fdv_usd: call_fdv,
            score: row.get(3)?,
            tx_5m: row.get(4)?,
            signers_5m: row.get(5)?,
            events: row.get(6)?,
            tag: row.get(7)?,
            peak_fdv,
            outcome,
        });
    }

    Ok(calls)
}

fn load_monitored_wallets(conn: &Connection) -> rusqlite::Result<Vec<WalletData>> {
    // Don’t filter out negative wallets if you want to detect “high-risk wallets”.
    // Keep it broad but still bounded.
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

fn load_wallets_in_calls(conn: &Connection) -> rusqlite::Result<HashMap<String, Vec<String>>> {
    // Early window only (last 5m before call). DISTINCT to avoid duplicates.
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

fn measure_missing_signers(conn: &Connection) -> rusqlite::Result<HashMap<String, i64>> {
    // calls table may not have uniq_sigs; measure only what exists (signers/events).
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

/// Try reading top1/top5 from call_wallet_fingerprints if it exists.
/// Returns None if table missing or no data for mint.
fn get_concentration_if_available(
    conn: &Connection,
    mint: &str,
    call_ts: i64,
) -> Option<(f64, f64)> {
    let sql = r#"
        SELECT top1_share, top5_share
        FROM call_wallet_fingerprints
        WHERE mint = ? AND call_ts = ?
        LIMIT 1
    "#;

    let mut stmt = conn.prepare(sql).ok()?;
    let out: rusqlite::Result<(f64, f64)> = stmt.query_row([mint, &call_ts.to_string()], |row| {
        Ok((row.get::<_, f64>(0)?, row.get::<_, f64>(1)?))
    });

    out.ok()
}

fn analyze(
    conn: &Connection,
    calls: &[CallData],
    wallets: &[WalletData],
    wallets_in_calls: &HashMap<String, Vec<String>>,
    missing: &HashMap<String, i64>,
) -> Vec<Recommendation> {
    let mut recs = Vec::new();

    if calls.is_empty() {
        recs.push(Recommendation {
            description: "No calls found yet".to_string(),
            suggested_adjustment: "Run bot longer to collect call data".to_string(),
        });
        return recs;
    }

    let winners: Vec<&CallData> = calls.iter().filter(|c| c.outcome == "winner").collect();
    let rugs: Vec<&CallData> = calls.iter().filter(|c| c.outcome == "rug").collect();

    // Winner FDV band suggestion
    if !winners.is_empty() {
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

        let avg_signers =
            winners.iter().map(|c| c.signers_5m as f64).sum::<f64>() / winners.len() as f64;
        recs.push(Recommendation {
            description: "Avg signers_5m in WINNER calls".to_string(),
            suggested_adjustment: format!(
                "avg_signers_5m≈{:.1} (consider using signer_strength around this)",
                avg_signers
            ),
        });
    } else {
        recs.push(Recommendation {
            description: "No WINNER calls in sample window".to_string(),
            suggested_adjustment: "Don’t retune bands yet; collect more data or widen peak window"
                .to_string(),
        });
    }

    // Missing signal counts (signers/events)
    let low_signers = *missing.get("low_signers").unwrap_or(&0);
    let low_events = *missing.get("low_events").unwrap_or(&0);
    recs.push(Recommendation {
        description: "Calls with tx_5m>50 but signers_5m=0".to_string(),
        suggested_adjustment: format!(
            "count={}. If high, strengthen spam guard + ensure Helius per-coin ingest is healthy.",
            low_signers
        ),
    });
    recs.push(Recommendation {
        description: "Calls with tx_5m>50 but events=0".to_string(),
        suggested_adjustment: format!(
            "count={}. If high, your event pipeline is dropping edges.",
            low_events
        ),
    });

    // Concentration heuristic from rugs (prefers call_wallet_fingerprints if present)
    if !rugs.is_empty() {
        let mut seen = 0usize;
        let mut top1_sum = 0.0;
        let mut top5_sum = 0.0;

        for c in &rugs {
            if let Some((top1, top5)) = get_concentration_if_available(conn, &c.mint, c.ts) {
                top1_sum += top1;
                top5_sum += top5;
                seen += 1;
            } else {
                // fallback: very rough proxy using unique wallets seen (NOT true concentration)
                // we keep it but do not pretend it’s “share”
                let uniq_wallets = wallets_in_calls.get(&c.mint).map(|v| v.len()).unwrap_or(0);
                if uniq_wallets > 0 {
                    // treat low uniq_wallets as “more concentrated” just for triage
                    top1_sum += 1.0 / (uniq_wallets as f64).max(1.0);
                    top5_sum += 5.0 / (uniq_wallets as f64).max(1.0);
                    seen += 1;
                }
            }
        }

        if seen > 0 {
            let avg_top1 = top1_sum / seen as f64;
            let avg_top5 = top5_sum / seen as f64;

            recs.push(Recommendation {
                description: "RUG concentration signal (avg top1/top5-ish)".to_string(),
                suggested_adjustment: format!(
                    "avg_top1≈{:.3}, avg_top5≈{:.3}. Consider gates like: top1>=0.22 OR top5>=0.55 unless override_ok.",
                    avg_top1, avg_top5
                ),
            });
        } else {
            recs.push(Recommendation {
                description: "RUG concentration could not be computed".to_string(),
                suggested_adjustment:
                    "Store call_wallet_fingerprints on each call for real top1/top5 shares"
                        .to_string(),
            });
        }
    }

    // High-risk wallets (now possible because we didn’t filter negatives out)
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

fn print_brain_report(
    calls: &[CallData],
    wallets: &[WalletData],
    missing: &HashMap<String, i64>,
    recs: &[Recommendation],
) {
    println!("\n=== ADAPTIVE BRAIN REPORT ===\n");
    println!(
        "Analyzed {} calls, {} wallets\n",
        calls.len(),
        wallets.len()
    );

    println!("Recent Calls (last 8):");
    for c in calls.iter().take(8) {
        let peak = c
            .peak_fdv
            .map(|v| format!("{:.0}", v))
            .unwrap_or_else(|| "-".to_string());
        println!(
            "Mint: {} | callFDV:${:.0} | peak:${} | outcome:{} | signers:{} | tx5:{} | ev:{} | tag:{:?}",
            c.mint, c.fdv_usd, peak, c.outcome, c.signers_5m, c.tx_5m, c.events, c.tag
        );
    }

    println!("\nSignal health (calls):");
    println!(
        "low_signers(tx>50 & signers=0)={} | low_events(tx>50 & events=0)={}",
        missing.get("low_signers").unwrap_or(&0),
        missing.get("low_events").unwrap_or(&0)
    );

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
